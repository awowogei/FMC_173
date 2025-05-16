use fmc::{
    bevy::math::DVec3,
    items::{ItemStack, Items},
    models::{AnimationPlayer, Model, ModelMap, Models},
    physics::{Collider, Physics},
    prelude::*,
    utils::Rng,
    world::chunk::ChunkPosition,
};

use crate::players::{Health, Inventory};

pub struct DroppedItemsPlugin;
impl Plugin for DroppedItemsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, pick_up_items)
            .add_systems(Update, spawn_model.in_set(DropItems));
    }
}

/// Order systems that drop blocks before this systemset to avoid 1-frame lag.
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub struct DropItems;

/// An item stack that is dropped on the ground.
#[derive(Component, Deref, DerefMut)]
#[require(Transform)]
pub struct DroppedItem(ItemStack);

impl DroppedItem {
    pub fn new(item_stack: ItemStack) -> Self {
        Self(item_stack)
    }
}

fn spawn_model(
    mut commands: Commands,
    models: Res<Models>,
    items: Res<Items>,
    mut dropped_items: Query<
        (Entity, &DroppedItem, Option<&Physics>, &mut Transform),
        Added<DroppedItem>,
    >,
    mut rng: Local<Rng>,
) {
    for (entity, dropped_item, maybe_physics, mut transform) in dropped_items.iter_mut() {
        let item_id = dropped_item.0.item().unwrap().id;
        let item_config = items.get_config(&item_id);
        let model_config = models.get_by_id(item_config.model_id);

        let mut aabb = model_config.aabb.clone();

        // There are two scales, one scales the model to have a volume and the other scales
        // it to be some height. We choose whatever makes it smaller.
        const HALF_VOLUME: f64 = 0.15 * 0.15 * 0.15;
        let half_volume = aabb.half_extents.x * aabb.half_extents.y * aabb.half_extents.z;
        let volume_scale = (HALF_VOLUME / half_volume).cbrt();
        const HALF_HEIGHT: f64 = 0.25;
        let y_scale = HALF_HEIGHT / aabb.half_extents.y;
        let scale = volume_scale.min(y_scale);

        transform.scale = DVec3::splat(scale);
        // Moving it down to create some constant spacing between the item and the ground.
        aabb.center.y -= 0.01 / scale;

        let mut animation_player = AnimationPlayer::default();
        let animation_index = model_config.animations.get("dropped").cloned();
        animation_player.set_idle_animation(animation_index);
        animation_player.set_move_animation(animation_index);

        let mut entity_commands = commands.entity(entity);

        entity_commands.insert((
            Model::Asset(item_config.model_id),
            animation_player,
            Collider::Aabb(aabb),
        ));

        if maybe_physics.is_none() {
            let random = rng.next_f32() * std::f32::consts::TAU;
            let velocity_x = random.sin() as f64 * 3.0;
            let velocity_z = random.cos() as f64 * 3.0;
            let velocity_y = 6.5;

            entity_commands.insert(Physics {
                velocity: DVec3::new(velocity_x, velocity_y, velocity_z),
                ..default()
            });
        }
    }
}

fn pick_up_items(
    mut commands: Commands,
    model_map: Res<ModelMap>,
    mut players: Query<(&GlobalTransform, &mut Inventory, &Health), Changed<GlobalTransform>>,
    mut dropped_items: Query<(Entity, &mut DroppedItem, &Transform)>,
) {
    for (player_position, mut player_inventory, health) in players.iter_mut() {
        if health.is_dead() {
            continue;
        }

        let chunk_position = ChunkPosition::from(player_position.translation());
        let item_entities = match model_map.get_entities(&chunk_position) {
            Some(e) => e,
            None => continue,
        };

        for item_entity in item_entities.iter() {
            if let Ok((entity, mut dropped_item, transform)) = dropped_items.get_mut(*item_entity) {
                if transform
                    .translation
                    .distance_squared(player_position.translation())
                    < 2.0
                {
                    // First test that the item can be picked up. This is to avoid triggering
                    // change detection for the inventory. If detection is triggered, it will send
                    // an interface update to the client. Can't pick up = spam
                    let mut capacity = false;
                    for item_stack in player_inventory.iter() {
                        if (item_stack.item() == dropped_item.item()
                            && item_stack.remaining_capacity() != 0)
                            || item_stack.is_empty()
                        {
                            capacity = true;
                            break;
                        }
                    }
                    if !capacity {
                        break;
                    }

                    // First try to fill item stacks that already have the item
                    for item_stack in player_inventory.iter_mut() {
                        if item_stack.item() == dropped_item.item() {
                            dropped_item.transfer_to(item_stack, u32::MAX);
                        }

                        if dropped_item.is_empty() {
                            break;
                        }
                    }

                    if dropped_item.is_empty() {
                        commands.entity(entity).despawn();
                        continue;
                    }

                    // Then go again and fill empty spots
                    for item_stack in player_inventory.iter_mut() {
                        if item_stack.is_empty() {
                            dropped_item.transfer_to(item_stack, u32::MAX);
                        }

                        if dropped_item.is_empty() {
                            break;
                        }
                    }

                    if dropped_item.is_empty() {
                        commands.entity(entity).despawn();
                        continue;
                    }
                }
            }
        }
    }
}
