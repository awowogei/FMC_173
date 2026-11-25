use fmc::{
    blocks::BlockId,
    players::{Player, Target, Targets},
    prelude::*,
    world::{BlockUpdate, WorldMap},
};

use super::{ItemAction, ItemEvents, Items};
use crate::world::blocks::Blocks;

pub struct SeedPlugin;
impl Plugin for SeedPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, register_seeds)
            .add_systems(Update, use_seeds.after(super::ItemSystems));
    }
}

fn register_seeds(mut commands: Commands, blocks: Res<Blocks>, items: Res<Items>) {
    let item_id = items.get_id("wheat_seeds").unwrap();
    commands.spawn((
        ItemEvents::new(item_id),
        SeedConfig {
            air: blocks.get_id("air"),
            soil: blocks.get_id("soil"),
        },
    ));
}

#[derive(Component)]
struct SeedConfig {
    pub air: BlockId,
    pub soil: BlockId,
}

fn use_seeds(
    world_map: Res<WorldMap>,
    blocks: Res<Blocks>,
    player_query: Query<&Targets, With<Player>>,
    mut seed_query: Query<(&mut ItemEvents, &SeedConfig), Changed<ItemEvents>>,
    mut block_update_writer: MessageWriter<BlockUpdate>,
) {
    let Ok((mut events, config)) = seed_query.single_mut() else {
        return;
    };

    for event in events.read() {
        if event.action != ItemAction::Use {
            continue;
        }

        let targets = player_query.get(event.player_entity).unwrap();

        let Some(Target::Block { block_position, .. }) =
            targets.get_first_block(|block_id| *block_id == config.soil)
        else {
            continue;
        };

        if let Some(above_block) = world_map.get_block(*block_position + IVec3::Y) {
            if above_block != config.air {
                continue;
            }
        } else {
            continue;
        }

        block_update_writer.write(BlockUpdate::Replace {
            position: *block_position + IVec3::Y,
            block_id: blocks.get_id("wheat_0"),
            block_state: None,
            block_data: None,
        });
    }
}
