use fmc::{
    audio::{AudioPlayer, Sound},
    blocks::{BlockId, BlockPosition},
    math::DVec3,
    models::{Model, ModelAssetId, ModelColor, Models},
    physics::{Collider, Physics},
    prelude::*,
    world::BlockUpdate,
};

use super::Blocks;
use crate::explosions::ExplosionEvent;

pub struct TntPlugin;
impl Plugin for TntPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<TntIgnition>()
            .add_systems(Startup, setup)
            .add_systems(Update, (ignite_tnt, tick_fuse));
    }
}

const EXPLOSION_POWER: f32 = 4.0;

#[derive(Message)]
pub struct TntIgnition {
    pub position: BlockPosition,
    pub velocity: Vec3,
    pub fuse: f32,
}

#[derive(Resource)]
struct TntConfig {
    air: BlockId,
    model_id: ModelAssetId,
    collider: Collider,
}

#[derive(Component)]
struct Tnt {
    fuse: Timer,
}

fn setup(mut commands: Commands, blocks: Res<Blocks>, models: Res<Models>) {
    let model = models.get_config_by_name("tnt").unwrap();
    commands.insert_resource(TntConfig {
        air: blocks.get_id("air"),
        model_id: model.id,
        collider: model.collider.clone(),
    });
}

fn ignite_tnt(
    mut commands: Commands,
    audio: AudioPlayer,
    config: Res<TntConfig>,
    mut ignite_reader: MessageReader<TntIgnition>,
    mut block_update_writer: MessageWriter<BlockUpdate>,
) {
    for ignite in ignite_reader.read() {
        block_update_writer.write(BlockUpdate::Replace {
            position: ignite.position,
            block_id: config.air,
            block_state: None,
            block_data: None,
        });

        let translation = ignite.position.as_dvec3() + DVec3::new(0.5, 0.0, 0.5);

        commands.spawn((
            Tnt {
                fuse: Timer::from_seconds(ignite.fuse, TimerMode::Once),
            },
            Model::Asset(config.model_id),
            config.collider.clone(),
            Physics {
                velocity: ignite.velocity.as_dvec3(),
                mass: 2.5,
                ..default()
            },
            Transform::from_translation(translation),
        ));

        audio.play_at(translation, Sound::new("fuse.ogg"));
    }
}

fn tick_fuse(
    mut commands: Commands,
    time: Res<Time>,
    mut primed_tnt: Query<(Entity, &mut Tnt, &Transform, &mut ModelColor)>,
    mut explosion_writer: MessageWriter<ExplosionEvent>,
) {
    for (entity, mut tnt, transform, mut color) in primed_tnt.iter_mut() {
        tnt.fuse.tick(time.delta());

        if tnt.fuse.is_finished() {
            explosion_writer.write(ExplosionEvent {
                position: transform.translation,
                power: EXPLOSION_POWER,
            });
            commands.entity(entity).despawn();
            continue;
        }

        let brightness = if tnt.fuse.elapsed_secs() % 0.5 > 0.25 {
            2.0
        } else {
            1.0
        };
        color.set("fuse", Vec4::new(brightness, brightness, brightness, 1.0));
    }
}
