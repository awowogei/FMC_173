use fmc::{
    audio::{AudioPlayer, Sound},
    blocks::BlockPosition,
    math::DVec3,
    particle_effects::{ParticleEffect, ParticleSpawner},
    prelude::*,
    random::Rng,
    world::{BlockUpdate, WorldMap},
};

use crate::{
    items::DroppedItem,
    world::blocks::{Blocks, tnt::TntIgnition},
};

pub struct ExplosionsPlugin;
impl Plugin for ExplosionsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ExplosionEvent>()
            .add_systems(Update, explode);
    }
}

#[derive(Message)]
pub struct ExplosionEvent {
    pub position: DVec3,
    pub power: f32,
}

// The max power an explosion can be given
const MAX_POWER: f32 = 5.0;
// Fraction of a block's own blast resistance that counts when deciding if it breaks
const BREAKING_FRACTION: f32 = 0.3;
// How much the strength can vary by direction
const STRENGTH_VARIATION: f32 = 0.3;
// +/- random noise added to each block's cost for the break check
const EDGE_JITTER: f32 = 0.5;
// Scale for the blast resistance. The total power loss is: distance + resistance * scale
const RESISTANCE_SCALE: f32 = 0.15;

fn reach(power: f32) -> i32 {
    (power * (1.0 + STRENGTH_VARIATION)).ceil() as i32 + 1
}

// Gives blast directions variable strength to introduce greater variance to craters than the jitter
// can provide.
struct DirectionalStrength([(Vec3, f32); 8]);

impl DirectionalStrength {
    fn new(rng: &mut Rng) -> Self {
        Self(std::array::from_fn(|_| {
            (Self::random_unit_vector(rng), rng.next_f32() * 2.0 - 1.0)
        }))
    }

    fn random_unit_vector(rng: &mut Rng) -> Vec3 {
        loop {
            let vector = Vec3::new(
                rng.next_f32() * 2.0 - 1.0,
                rng.next_f32() * 2.0 - 1.0,
                rng.next_f32() * 2.0 - 1.0,
            );
            let length_squared = vector.length_squared();
            if length_squared > 0.01 && length_squared <= 1.0 {
                return vector / length_squared.sqrt();
            }
        }
    }

    fn apply(&self, power: f32, direction: Vec3) -> f32 {
        let direction = direction.normalize_or_zero();
        let variation: f32 = self
            .0
            .iter()
            // The dot product here is just the cosine, both are length = 1
            .map(|(lobe, weight)| weight * direction.dot(*lobe).max(0.0).powi(4))
            .sum();
        power * (1.0 + STRENGTH_VARIATION * variation.clamp(-1.0, 1.0))
    }
}

fn explode(
    mut commands: Commands,
    audio: AudioPlayer,
    particles: ParticleSpawner,
    blocks: Res<Blocks>,
    world_map: Res<WorldMap>,
    mut explosion_events: MessageReader<ExplosionEvent>,
    mut block_update_writer: MessageWriter<BlockUpdate>,
    mut ignite_writer: MessageWriter<TntIgnition>,
    mut rng: Local<Rng>,
    mut offsets: Local<Vec<IVec3>>,
    mut path_resistance: Local<Vec<f32>>,
) {
    // Instead of handling a queue for which block to visit next each explosion we precompute it.
    // XXX: If MAX_POWER is increased, the memory this uses should be weighed against the performance
    // cost.
    if offsets.is_empty() {
        let radius = reach(MAX_POWER);
        for x in -radius..=radius {
            for y in -radius..=radius {
                for z in -radius..=radius {
                    let offset = IVec3::new(x, y, z);
                    if offset.length_squared() <= radius * radius {
                        offsets.push(offset);
                    }
                }
            }
        }
        offsets.sort_by_key(|offset| offset.length_squared());
    }

    let air = blocks.get_id("air");
    let tnt = blocks.get_id("tnt");

    for explosion in explosion_events.read() {
        let power = explosion.power.clamp(0.0, MAX_POWER);
        let radius = reach(power);
        let size = (2 * radius + 1) as usize;
        let index = |offset: IVec3| {
            let shifted = (offset + radius).as_uvec3();
            (shifted.x as usize * size + shifted.y as usize) * size + shifted.z as usize
        };

        path_resistance.clear();
        path_resistance.resize(size.pow(3), f32::INFINITY);

        let directional_strength = DirectionalStrength::new(&mut rng);

        let center = BlockPosition::from(explosion.position);
        let center_offset = (center.as_dvec3() + 0.5 - explosion.position).as_vec3();

        for &offset in offsets.iter() {
            if offset.length_squared() > radius.pow(2) {
                break;
            }

            // Find the path of least resistance the explosion can take to this offset
            let previous = if offset == IVec3::ZERO {
                0.0
            } else {
                let mut cheapest = f32::INFINITY;
                for axis in 0..3 {
                    if offset[axis] != 0 {
                        let mut predecessor = offset;
                        predecessor[axis] -= offset[axis].signum();
                        cheapest = cheapest.min(path_resistance[index(predecessor)]);
                    }
                }
                cheapest
            };

            // The explosion couldn't reach this position
            if previous.is_infinite() {
                continue;
            }

            let position = center + offset;
            let Some(block_id) = world_map.get_block(position) else {
                continue;
            };

            let block_config = blocks.get_config(&block_id);
            let cost = block_config.blast_resistance * RESISTANCE_SCALE;

            // Because of the manhattan walk we have to adjust the cost downwards when it's a diagonal to get
            // a more spherical result.
            let manhattan = offset.abs().element_sum();
            let staircase_correction = if manhattan == 0 {
                1.0
            } else {
                (offset.length_squared() as f32).sqrt() / manhattan as f32
            };

            let to_block = offset.as_vec3() + center_offset;
            let strength = directional_strength.apply(power, to_block) - to_block.length();

            if block_id == tnt {
                ignite_writer.write(TntIgnition {
                    position,
                    velocity: to_block.normalize_or_zero() * strength.max(0.0) * 10.0,
                    fuse: 0.5 + rng.next_f32(),
                });
            } else if block_id != air {
                let jitter = (rng.next_f32() * 2.0 - 1.0) * EDGE_JITTER;
                let breaks = strength
                    - (previous + cost * BREAKING_FRACTION) * staircase_correction
                    + jitter
                    > 0.0;
                if breaks {
                    block_update_writer.write(BlockUpdate::Replace {
                        position,
                        block_id: air,
                        block_state: None,
                        block_data: None,
                    });

                    if rng.next_f32() < 0.3
                        && let Some(item_stack) = block_config.drop(None, &mut rng)
                    {
                        commands.spawn((
                            DroppedItem::new(item_stack),
                            Transform::from_translation(position.as_dvec3() + DVec3::splat(0.5)),
                        ));
                    }
                }
            }

            let total = previous + cost;
            let passes_through = strength - total * staircase_correction > 0.0;
            if passes_through {
                path_resistance[index(offset)] = total;
            }
        }

        audio.play_at(explosion.position, Sound::new("explosion.ogg"));

        // White explosion particles
        particles.spawn(
            explosion.position,
            ParticleEffect::new("explosion_white", "particles/explosion2.png"),
        );

        // Gray explosion particles
        particles.spawn(
            explosion.position,
            ParticleEffect::new("explosion_gray", "particles/explosion2.png")
                .with_color(Vec4::splat(0.6).with_w(1.0)),
        );

        // Black particles
        particles.spawn(
            explosion.position,
            ParticleEffect::new("explosion_black", "particles/explosion2.png")
                .with_color(Vec4::splat(0.1).with_w(1.0)),
        );
    }
}
