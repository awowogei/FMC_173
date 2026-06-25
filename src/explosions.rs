use fmc::{
    bevy::math::DVec3,
    blocks::{BlockPosition, Blocks},
    networking::Server,
    particle_effects::ParticleEffects,
    prelude::*,
    protocol::messages,
    world::{BlockUpdate, ChunkSubscriptions, chunk::ChunkPosition},
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
    pub radius: u32,
}

// TODO: See https://minecraft.wiki/w/Explosion for how to actually do explosions
fn explode(
    net: Res<Server>,
    particle_effects: Res<ParticleEffects>,
    chunk_subscriptions: Res<ChunkSubscriptions>,
    mut explosion_events: MessageReader<ExplosionEvent>,
    mut block_update_writer: MessageWriter<BlockUpdate>,
) {
    for explosion in explosion_events.read() {
        let air = Blocks::get().get_id("air");
        let radius = 3;
        for x in -radius..radius {
            for z in -radius..radius {
                for y in -radius..radius {
                    let position = BlockPosition::new(x, y, z);
                    if position.length_squared() > radius * radius {
                        continue;
                    }

                    block_update_writer.write(BlockUpdate::Replace {
                        position: BlockPosition::from(explosion.position) + position,
                        block_id: air,
                        block_state: None,
                        block_data: None,
                    });
                }
            }
        }

        let chunk_position = ChunkPosition::from(explosion.position);
        let Some(subscribers) = chunk_subscriptions.get_subscribers(&chunk_position) else {
            continue;
        };

        net.send_many(
            subscribers,
            messages::Sound {
                position: Some(explosion.position),
                volume: 1.0,
                speed: 1.0,
                sound: "explosion.ogg".to_owned(),
            },
        );

        // White explosion particles
        net.send_many(
            subscribers,
            messages::ParticleEffect {
                id: particle_effects.get_id("explosion_white").unwrap(),
                position: explosion.position,
                rotation: Quat::IDENTITY,
                texture: "particles/explosion2.png".to_owned(),
                color: Vec4::ONE,
            },
        );

        // Gray explosion particles
        net.send_many(
            subscribers,
            messages::ParticleEffect {
                id: particle_effects.get_id("explosion_gray").unwrap(),
                position: explosion.position,
                rotation: Quat::IDENTITY,
                texture: "particles/explosion2.png".to_owned(),
                color: Vec4::splat(0.6).with_w(1.0),
            },
        );

        // Black particles
        net.send_many(
            subscribers,
            messages::ParticleEffect {
                id: particle_effects.get_id("explosion_black").unwrap(),
                position: explosion.position,
                rotation: Quat::IDENTITY,
                texture: "particles/explosion2.png".to_owned(),
                color: Vec4::splat(0.1).with_w(1.0),
            },
        );
    }
}
