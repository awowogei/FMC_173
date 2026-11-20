use fmc::{
    blocks::BlockId,
    players::{Player, Target, Targets},
    prelude::*,
    random::Rng,
};

use super::{ItemRegistry, ItemUses, Items};
use crate::world::blocks::{Blocks, tnt::TntIgnition};

pub struct FlintAndSteelPlugin;
impl Plugin for FlintAndSteelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, register_flint_and_steel)
            .add_systems(Update, use_flint_and_steel.after(super::ItemUseSystems));
    }
}

#[derive(Component)]
struct FlintAndSteelConfig {
    tnt: BlockId,
}

fn register_flint_and_steel(
    mut commands: Commands,
    blocks: Res<Blocks>,
    items: Res<Items>,
    mut usable_items: ResMut<ItemRegistry>,
) {
    usable_items.insert(
        items.get_id("flint_and_steel").unwrap(),
        commands
            .spawn((
                ItemUses::default(),
                FlintAndSteelConfig {
                    tnt: blocks.get_id("tnt"),
                },
            ))
            .id(),
    );
}

fn use_flint_and_steel(
    player_query: Query<&Targets, With<Player>>,
    mut uses: Query<(&mut ItemUses, &FlintAndSteelConfig), Changed<ItemUses>>,
    mut ignite_writer: MessageWriter<TntIgnition>,
    mut rng: Local<Rng>,
) {
    let Ok((mut uses, config)) = uses.single_mut() else {
        return;
    };

    for player_entity in uses.read() {
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
