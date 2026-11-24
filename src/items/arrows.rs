use fmc::{
    blocks::BlockPosition,
    math::{DQuat, DVec3},
    models::Model,
    networking::Server,
    physics::{Collider, Physics},
    players::Player,
    prelude::*,
    protocol::messages,
    random::Rng,
    visibility::EntityMap,
    world::{ChangedBlockEvent, WorldMap, chunk::ChunkPosition},
};
use std::collections::{HashMap, HashSet};

use super::{ItemRegistry, ItemUses, Items};
use crate::{
    mobs::{Mob, MobDamageEvent, MobHealth},
    players::{HealEvent, Inventory, PlayerDamageEvent},
    world::blocks::Blocks,
};

pub struct ArrowPlugin;
impl Plugin for ArrowPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, arrows);
    }
}

#[derive(Component)]
pub struct Arrow {
    despawn_timer: Option<Timer>,
    stuck_position: Option<BlockPosition>,
    velocity: DVec3,
}

impl Arrow {
    pub const GRAVITY: f64 = 56.0;
    pub const SPEED: f64 = 40.0;
    const SPIN_SPEED: f64 = std::f64::consts::TAU * 2.0;
    const DAMAGE: u32 = 4;
    const HEAD_OFFSET: f64 = 0.15;
    const HIT_SOUNDS: [&str; 3] = ["arrow_1.ogg", "arrow_2.ogg", "arrow_3.ogg"];

    pub fn new(velocity: DVec3) -> Self {
        Self {
            despawn_timer: None,
            stuck_position: None,
            velocity,
        }
    }

    fn start_despawn_timer(&mut self) {
        self.despawn_timer = Some(Timer::from_seconds(120.0, TimerMode::Once));
    }
}

#[derive(Default)]
struct StuckArrows {
    arrows: HashMap<BlockPosition, HashSet<Entity>>,
}

impl StuckArrows {
    fn insert(&mut self, block_position: BlockPosition, entity: Entity) {
        self.arrows
            .entry(block_position)
            .or_default()
            .insert(entity);
    }

    fn remove(&mut self, block_position: BlockPosition, entity: Entity) {
        let arrows = self.arrows.get_mut(&block_position).unwrap();
        arrows.remove(&entity);
        if arrows.is_empty() {
            self.arrows.remove(&block_position);
        }
    }
}

fn arrows(
    mut commands: Commands,
    time: Res<Time>,
    net: Res<Server>,
    blocks: Res<Blocks>,
    world_map: Res<WorldMap>,
    entity_map: Res<EntityMap>,
    mut arrow_query: Query<(Entity, &mut Arrow, &mut Transform)>,
    model_query: Query<
        (Entity, &Transform, &Collider, Has<Player>, Has<Mob>),
        (Without<Arrow>, With<Model>),
    >,
    mut block_updates: MessageReader<ChangedBlockEvent>,
    mut stuck_arrows: Local<StuckArrows>,
    mut mob_physics: Query<(&mut Physics, &MobHealth)>,
    mut player_damage_events: MessageWriter<PlayerDamageEvent>,
    mut mob_damage_events: MessageWriter<MobDamageEvent>,
    mut rng: Local<Rng>,
) {
    'arrows: for (arrow_entity, mut arrow, mut transform) in arrow_query.iter_mut() {
        if let Some(timer) = &mut arrow.despawn_timer {
            timer.tick(time.delta());

            if timer.just_finished() {
                commands.entity(arrow_entity).despawn();
                if let Some(stuck_position) = arrow.stuck_position.take() {
                    stuck_arrows.remove(stuck_position, arrow_entity);
                }
            }

            continue;
        }

        if arrow.velocity != DVec3::ZERO {
            // Make it spin around the shaft, with some variation so that each arrow shot the same
            // distance doesn't hit the ground the same + looks a little different in the air.
            let spin_factor = (arrow.velocity.length() / 10.0).min(1.0).powi(2);
            let spin_variation = 0.5 + Rng::new(arrow_entity.to_bits()).next_f32() as f64;
            let spin = time.delta_secs_f64() * Arrow::SPIN_SPEED * spin_factor * spin_variation;

            // We do the quaternion math by hand since looking_to applies an unpredictable twist
            // when the arrow goes vertical. We only want rotation around the horizontal axis when
            // this happens so it drops cleanly.
            let turn = DQuat::from_rotation_arc(transform.forward(), arrow.velocity.normalize());
            transform.rotation = DQuat::IDENTITY
                // Apply a softer turn so that when it unsticks from things it will make it look like
                // the heavy head leads it. Otherwise it would point downwards instantly.
                // The model's origin also has to be placed inside the arrow head for this effect to work.
                .rotate_towards(turn, std::f64::consts::PI * time.delta_secs_f64())
                * transform.rotation
                * DQuat::from_rotation_z(spin);
        }

        let max_distance = (arrow.velocity * time.delta_secs_f64()).length();

        for chunk_position in ChunkPosition::from(transform.translation).neighbourhood() {
            for (model_entity, model_transform, collider, is_player, is_mob) in
                model_query.iter_many(entity_map.iter_entities(&chunk_position))
            {
                let Some((distance, _)) =
                    collider.ray_intersection(model_transform, &transform, false)
                else {
                    continue;
                };
                if distance > max_distance {
                    continue;
                }

                let horizontal = arrow.velocity.xz().normalize_or_zero() * 5.0;
                let knock_back = DVec3::new(horizontal.x, 4.0, horizontal.y);

                let forward = transform.forward();
                transform.translation += forward * (distance + Arrow::HEAD_OFFSET);
                let inverse_rotation = model_transform.rotation.inverse();
                transform.translation =
                    inverse_rotation * (transform.translation - model_transform.translation);
                transform.rotation = inverse_rotation * transform.rotation;
                commands.entity(model_entity).add_child(arrow_entity);

                arrow.start_despawn_timer();
                arrow.velocity = DVec3::ZERO;

                if is_player {
                    player_damage_events.write(PlayerDamageEvent {
                        player_entity: model_entity,
                        damage: Arrow::DAMAGE,
                        knock_back: Some(knock_back),
                    });
                } else if is_mob {
                    mob_damage_events.write(MobDamageEvent {
                        mob_entity: model_entity,
                        damage: Arrow::DAMAGE,
                    });

                    if let Ok((mut physics, health)) = mob_physics.get_mut(model_entity)
                        && !health.is_invincible()
                    {
                        physics.velocity = knock_back;
                    }
                }

                continue 'arrows;
            }
        }

        let mut friction = DVec3::ZERO;
        let mut raycast = world_map.raycast(&transform, max_distance);
        while let Some(block_id) = raycast.next_block() {
            let block_config = blocks.get_config(&block_id);
            if let Some(drag) = block_config.drag() {
                friction = friction.max(drag);
                continue;
            }

            let forward = transform.forward();
            // Pushes the arrowhead into the surface
            transform.translation += forward * (raycast.distance() + Arrow::HEAD_OFFSET);
            arrow.start_despawn_timer();
            arrow.stuck_position = Some(raycast.position());
            stuck_arrows.insert(raycast.position(), arrow_entity);
            arrow.velocity = DVec3::ZERO;

            let sound = Arrow::HIT_SOUNDS[rng.next_usize() % Arrow::HIT_SOUNDS.len()];
            net.broadcast(messages::Sound {
                position: Some(transform.translation),
                volume: 1.0,
                speed: 1.0,
                sound: sound.to_owned(),
            });
            break;
        }

        transform.translation += arrow.velocity * time.delta_secs_f64();

        let mass = 10.0;
        let gravity_factor = (arrow.velocity.length() / 10.0).clamp(0.48, 1.0);
        arrow.velocity.y -= Arrow::GRAVITY * gravity_factor * time.delta_secs_f64();
        arrow.velocity *= (-friction / mass * time.delta_secs_f64()).exp()
    }

    for block_update in block_updates.read() {
        if let Some(arrows) = stuck_arrows.arrows.remove(&block_update.position) {
            for entity in arrows {
                let (_, mut arrow, _) = arrow_query.get_mut(entity).unwrap();
                arrow.despawn_timer = None;
                arrow.stuck_position = None;
            }
        }
    }
}
