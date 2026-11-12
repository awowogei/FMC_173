use fmc::{
    blocks::Blocks,
    items::{ItemStack, Items},
    players::{Player, Target, Targets},
    prelude::*,
    world::{BlockUpdate, WorldMap},
};

use crate::{players::Inventory, world::blocks::water::Water};

use super::{ItemRegistry, ItemUses};

pub struct BucketPlugin;
impl Plugin for BucketPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, register_buckets).add_systems(
            Update,
            (use_bucket, use_water_bucket).after(super::ItemUseSystems),
        );
    }
}

#[derive(Component)]
struct Bucket;

#[derive(Component)]
struct WaterBucket;

fn register_buckets(
    mut commands: Commands,
    items: Res<Items>,
    mut usable_items: ResMut<ItemRegistry>,
) {
    usable_items.insert(
        items.get_id("bucket").unwrap(),
        commands.spawn((ItemUses::default(), Bucket)).id(),
    );
    usable_items.insert(
        items.get_id("water_bucket").unwrap(),
        commands.spawn((ItemUses::default(), WaterBucket)).id(),
    );
}

fn use_bucket(
    water: Res<Water>,
    world_map: Res<WorldMap>,
    items: Res<Items>,
    mut player_query: Query<(&Targets, &mut Inventory), With<Player>>,
    mut bucket_uses: Query<&mut ItemUses, (With<Bucket>, Changed<ItemUses>)>,
    mut block_update_writer: MessageWriter<BlockUpdate>,
) {
    let Ok(mut uses) = bucket_uses.single_mut() else {
        return;
    };

    for player_entity in uses.read() {
        let (targets, mut inventory) = player_query.get_mut(player_entity).unwrap();

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
            block_id: Blocks::get().get_id("air"),
            block_state: None,
            block_data: None,
        });

        let water_bucket = items.get_config_by_name("water_bucket").unwrap();
        *inventory.held_item_stack_mut() = ItemStack::new(water_bucket, 1);
    }
}

fn use_water_bucket(
    water: Res<Water>,
    world_map: Res<WorldMap>,
    items: Res<Items>,
    mut player_query: Query<(&Targets, &mut Inventory), With<Player>>,
    mut water_bucket_uses: Query<&mut ItemUses, (With<WaterBucket>, Changed<ItemUses>)>,
    mut block_update_writer: MessageWriter<BlockUpdate>,
) {
    let Ok(mut uses) = water_bucket_uses.single_mut() else {
        return;
    };

    let blocks = Blocks::get();

    for player_entity in uses.read() {
        let (targets, mut inventory) = player_query.get_mut(player_entity).unwrap();

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
        *inventory.held_item_stack_mut() = ItemStack::new(bucket, 1);
    }
}
