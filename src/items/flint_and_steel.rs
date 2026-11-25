use fmc::{
    blocks::BlockId,
    players::{Player, Target, Targets},
    prelude::*,
    random::Rng,
};

use super::{ItemEvents, Items};
use crate::world::blocks::{Blocks, tnt::TntIgnition};

pub struct FlintAndSteelPlugin;
impl Plugin for FlintAndSteelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, register_flint_and_steel)
            .add_systems(Update, use_flint_and_steel.after(super::ItemSystems));
    }
}

#[derive(Component)]
struct FlintAndSteelConfig {
    tnt: BlockId,
}

fn register_flint_and_steel(mut commands: Commands, blocks: Res<Blocks>, items: Res<Items>) {
    let item_id = items.get_id("flint_and_steel").unwrap();
    commands.spawn((
        ItemEvents::new(item_id),
        FlintAndSteelConfig {
            tnt: blocks.get_id("tnt"),
        },
    ));
}

fn use_flint_and_steel(
    player_query: Query<&Targets, With<Player>>,
    mut flint_and_steel_query: Query<(&mut ItemEvents, &FlintAndSteelConfig), Changed<ItemEvents>>,
    mut ignite_writer: MessageWriter<TntIgnition>,
    mut rng: Local<Rng>,
) {
    let Ok((mut events, config)) = flint_and_steel_query.single_mut() else {
        return;
    };

    for player_entity in events.read() {
        let targets = player_query.get(player_entity).unwrap();

        let Some(Target::Block { block_position, .. }) =
            targets.get_first_block(|block_id| *block_id == config.tnt)
        else {
            continue;
        };

        let angle = rng.next_f32() * std::f32::consts::TAU;
        let velocity = Vec3::new(angle.sin() * 0.5, 4.0, angle.cos() * 0.5);

        ignite_writer.write(TntIgnition {
            position: *block_position,
            velocity,
            fuse: 4.0,
        });
    }
}
