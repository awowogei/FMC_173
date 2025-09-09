use fmc::{
    bevy::math::DVec3,
    items::{ItemStack, Items},
    models::{AnimationPlayer, Model, ModelMap, Models},
    networking::Server,
    physics::{Collider, Physics},
    players::Camera,
    prelude::*,
    protocol::messages,
    utils::Rng,
    world::{ChunkSubscriptions, chunk::ChunkPosition},
};

use crate::players::{Health, Inventory};

pub struct DroppedItemsPlugin;
impl Plugin for DroppedItemsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, item_pickup)
            .add_systems(Update, spawn_model.in_set(DropItems));
    }
}

/// Order systems that drop blocks before this systemset to avoid 1-frame lag.
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub struct DropItems;

/// An item stack that is dropped on the ground.
#[derive(Component)]
#[require(Transform)]
pub struct DroppedItem {
    stack: ItemStack,
    drop_time: std::time::Instant,
    pickup_delay: std::time::Duration,
}

impl DroppedItem {
    pub fn new(item_stack: ItemStack) -> Self {
        Self {
            stack: item_stack,
            drop_time: std::time::Instant::now(),
            pickup_delay: std::time::Duration::from_secs_f32(0.5),
        }
    }

    pub fn with_delay(mut self, delay: f32) -> Self {
        self.pickup_delay = std::time::Duration::from_secs_f32(delay);
        self
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
        let item_id = dropped_item.stack.item().unwrap().id;
        let item_config = items.get_config(&item_id);
        let model_config = models.get_config(&item_config.model_id);

        let mut aabb = model_config.collider.as_aabb();

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
            Collider::Single(aabb),
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

// TODO: For some reason when you pick up items their animation is overwritten. You'd assume this
// is because it changes the transform, but on the client side the entity that is animated is
// a child of the model entity. This might be related to how there is a small jitter in the
// animation as the model is spawned.
fn item_pickup(
    mut commands: Commands,
    net: Res<Server>,
    model_map: Res<ModelMap>,
    chunk_subscriptions: Res<ChunkSubscriptions>,
    mut players: Query<(&GlobalTransform, &mut Inventory, &Health, &Camera)>,
    mut dropped_items: Query<(Entity, &mut DroppedItem, &mut Physics, &Transform)>,
) {
    let now = std::time::Instant::now();

    for (player_transform, mut player_inventory, health, camera) in players.iter_mut() {
        if health.is_dead() {
            continue;
        }

        // Some point towards the torso of the player we want the item to move towards when picked
        // up.
        let player_position = player_transform.translation() + camera.translation * 0.8;

        let neighbourhood = ChunkPosition::from(player_transform.translation()).neighbourhood();
        let item_entities = neighbourhood
            .iter()
            .flat_map(|chunk_position| model_map.get_entities(chunk_position))
            // Second flatten for &HashSet<Entity> -> &Entity
            .flatten();

        for item_entity in item_entities {
            let Ok((entity, mut dropped_item, mut physics, item_transform)) =
                dropped_items.get_mut(*item_entity)
            else {
                continue;
            };

            if now.duration_since(dropped_item.drop_time) < dropped_item.pickup_delay {
                continue;
            }

            let distance_squared = item_transform.translation.distance_squared(player_position);

            if distance_squared >= 4.0 {
                continue;
            }

            // First test that the item can be picked up. This is primarily to prevent triggering
            // change detection for the inventory, which would cause and interface update, but also
            // so the item doesn't move unless there's room in the inventory.
            let mut has_capacity = false;
            for item_stack in player_inventory.iter() {
                if (item_stack.item() == dropped_item.stack.item()
                    && item_stack.remaining_capacity() != 0)
                    || item_stack.is_empty()
                {
                    has_capacity = true;
                    break;
                }
            }
            if !has_capacity {
                break;
            }

            // Move the item towards the player
            physics.velocity = (player_position - item_transform.translation).normalize() * 10.0;

            // Pick up when it's just close enough not to disturb the camera view
            if distance_squared < 0.1 {
                if let Some(subscribers) = chunk_subscriptions
                    .get_subscribers(&ChunkPosition::from(item_transform.translation))
                {
                    net.send_many(
                        subscribers,
                        messages::Sound {
                            position: Some(player_position),
                            volume: 0.05,
                            speed: 1.5,
                            sound: "pickup.ogg".to_owned(),
                        },
                    );
                }

                // TODO: Auto-filling a slot in the inventory should be a method on Inventory.
                // It will be done other places.
                //
                // First try to fill item stacks that already have the item
                for item_stack in player_inventory.iter_mut() {
                    if item_stack.item() == dropped_item.stack.item() {
                        dropped_item.stack.transfer_to(item_stack, u32::MAX);
                    }

                    if dropped_item.stack.is_empty() {
                        break;
                    }
                }

                if dropped_item.stack.is_empty() {
                    commands.entity(entity).despawn();
                    continue;
                }

                // Then go again and fill empty spots
                for item_stack in player_inventory.iter_mut() {
                    if item_stack.is_empty() {
                        dropped_item.stack.transfer_to(item_stack, u32::MAX);
                    }

                    if dropped_item.stack.is_empty() {
                        break;
                    }
                }

                if dropped_item.stack.is_empty() {
                    commands.entity(entity).despawn();
                    continue;
                }
            }
        }
    }
}
