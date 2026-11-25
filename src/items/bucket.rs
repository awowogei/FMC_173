use fmc::{
    items::ItemStack,
    players::{Player, Target, Targets},
    prelude::*,
    world::{BlockUpdate, WorldMap},
};

use crate::{
    players::Inventory,
    world::blocks::{Blocks, water::Water},
};

use super::{ItemAction, ItemEvents, Items};

pub struct BucketPlugin;
impl Plugin for BucketPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, register_buckets).add_systems(
            Update,
            (use_bucket, use_water_bucket).after(super::ItemSystems),
        );
    }
}

#[derive(Component)]
struct Bucket;

#[derive(Component)]
struct WaterBucket;

fn register_buckets(mut commands: Commands, items: Res<Items>) {
    let item_id = items.get_id("bucket").unwrap();
    commands.spawn((ItemEvents::new(item_id), Bucket));

    let item_id = items.get_id("water_bucket").unwrap();
    commands.spawn((ItemEvents::new(item_id), WaterBucket));
}

fn use_bucket(
    water: Res<Water>,
    blocks: Res<Blocks>,
    world_map: Res<WorldMap>,
    items: Res<Items>,
    mut player_query: Query<(&Targets, &mut Inventory), With<Player>>,
    mut bucket_query: Query<&mut ItemEvents, (With<Bucket>, Changed<ItemEvents>)>,
    mut block_update_writer: MessageWriter<BlockUpdate>,
) {
    let Ok(mut events) = bucket_query.single_mut() else {
        return;
    };

    for event in events.read() {
        if event.action != ItemAction::Use {
            continue;
        }

        let (targets, mut inventory) = player_query.get_mut(event.player_entity).unwrap();

        let Some(source_position) = targets.iter().find_map(|target| match target {
            Target::Block {
                block_position,
                block_id,
                ..
            } if water.is_source(*block_id, world_map.get_block_state(*block_position)) => {
                Some(*block_position)
            }
            _ => None,
        }) else {
            continue;
        };

        block_update_writer.write(BlockUpdate::Replace {
            position: source_position,
            block_id: blocks.get_id("air"),
            block_state: None,
            block_data: None,
        });

        let water_bucket = items.get_config_by_name("water_bucket").unwrap();
        *inventory.held_item_stack_mut() = ItemStack::new(water_bucket.id, 1);
    }
}

fn use_water_bucket(
    water: Res<Water>,
    blocks: Res<Blocks>,
    world_map: Res<WorldMap>,
    items: Res<Items>,
    mut player_query: Query<(&Targets, &mut Inventory), With<Player>>,
    mut water_bucket_query: Query<&mut ItemEvents, (With<WaterBucket>, Changed<ItemEvents>)>,
    mut block_update_writer: MessageWriter<BlockUpdate>,
) {
    let Ok(mut events) = water_bucket_query.single_mut() else {
        return;
    };

    for event in events.read() {
        if event.action != ItemAction::Use {
            continue;
        }

        let (targets, mut inventory) = player_query.get_mut(event.player_entity).unwrap();

        let Some(Target::Block {
            block_position,
            block_id,
            block_face,
            ..
        }) = targets.get_first_block(|block_id| blocks.get_config(block_id).is_solid())
        else {
            continue;
        };

        let position = if blocks.get_config(block_id).replaceable {
            *block_position
        } else {
            let adjacent = block_face.shift_position(*block_position);
            let Some(adjacent_id) = world_map.get_block(adjacent) else {
                continue;
            };
            if !blocks.get_config(&adjacent_id).replaceable {
                continue;
            }
            adjacent
        };

        let (water_id, water_state) = water.source_block();
        block_update_writer.write(BlockUpdate::Replace {
            position,
            block_id: water_id,
            block_state: water_state,
            block_data: None,
        });

        let bucket = items.get_config_by_name("bucket").unwrap();
        *inventory.held_item_stack_mut() = ItemStack::new(bucket.id, 1);
    }
}
