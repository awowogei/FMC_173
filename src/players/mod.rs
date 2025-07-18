use std::collections::HashMap;

use fmc::{
    bevy::{
        ecs::query::QueryData,
        math::{DQuat, DVec3},
    },
    blocks::{BlockPosition, Blocks},
    database::Database,
    interfaces::{
        HeldInterfaceStack, InterfaceEventRegistration, InterfaceEvents, RegisterInterfaceNode,
    },
    items::ItemStack,
    models::{AnimationPlayer, Model, Models},
    networking::{NetworkEvent, NetworkMessage, Server},
    physics::{Collider, Physics},
    players::{Camera, Player},
    prelude::*,
    protocol::messages,
    world::{
        chunk::{Chunk, ChunkPosition},
        WorldMap,
    },
};
use serde::{Deserialize, Serialize};

use crate::{
    items::{crafting::CraftingGrid, DroppedItem},
    settings::Settings,
    world::WorldProperties,
};

use self::health::HealthBundle;

mod hand;
mod health;
mod inventory_interface;
mod movement;

pub use hand::{HandHits, HandInteractions};
pub use health::{HealEvent, Health, PlayerDamageEvent};

pub struct PlayerPlugin;
impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<RespawnEvent>()
            .add_plugins(inventory_interface::InventoryInterfacePlugin)
            .add_plugins(health::HealthPlugin)
            .add_plugins(hand::HandPlugin)
            .add_plugins(movement::MovementPlugin)
            .add_systems(
                Update,
                (
                    on_gamemode_update,
                    handle_gui_settings,
                    (add_players, ApplyDeferred).chain(),
                    respawn_players,
                    rotate_player_model,
                    discard_items.after(InterfaceEventRegistration),
                ),
            )
            // Save player after all remaining events have been handled. Avoid dupes and other
            // unexpected behaviour.
            .add_systems(PostUpdate, save_player_data_on_disconnect)
            .add_systems(
                Last,
                save_player_data_on_shutdown.run_if(on_event::<AppExit>),
            );
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum GameMode {
    #[default]
    Survival,
    Creative,
    Spectator,
}

#[derive(Component, Serialize, Deserialize, Deref, DerefMut, Clone)]
pub struct Inventory {
    #[deref]
    inventory: Vec<ItemStack>,
    equipped_item: usize,
}

impl Default for Inventory {
    fn default() -> Self {
        let capacity = 36;
        let mut inventory = Vec::with_capacity(capacity);
        inventory.resize_with(capacity, ItemStack::default);

        Self {
            inventory,
            equipped_item: 0,
        }
    }
}

impl Inventory {
    pub fn held_item_stack(&self) -> &ItemStack {
        &self[self.equipped_item]
    }

    pub fn held_item_stack_mut(&mut self) -> &mut ItemStack {
        let index = self.equipped_item;
        &mut self[index]
    }
}

// TODO: Move this into Inventory, no clue why I separated them
//
/// The equipment the
#[derive(Component, Default, Serialize, Deserialize, Clone)]
pub struct Equipment {
    pub helmet: ItemStack,
    pub chestplate: ItemStack,
    pub leggings: ItemStack,
    pub boots: ItemStack,
}

/// Components a player consists of
#[derive(Bundle)]
pub struct PlayerBundle {
    pub transform: Transform,
    pub camera: Camera,
    pub aabb: Collider,
    pub inventory: Inventory,
    pub equipment: Equipment,
    pub crafting_table: CraftingGrid,
    pub health_bundle: HealthBundle,
    pub game_mode: GameMode,
}

impl PlayerBundle {
    fn new(game_mode: GameMode) -> Self {
        Self {
            game_mode,
            ..default()
        }
    }
}

impl Default for PlayerBundle {
    fn default() -> Self {
        Self {
            transform: Transform::default(),
            camera: Camera::default(),
            aabb: Collider::from_min_max(DVec3::new(-0.3, 0.0, -0.3), DVec3::new(0.3, 1.8, 0.3)),
            inventory: Inventory::default(),
            equipment: Equipment::default(),
            crafting_table: CraftingGrid::with_size(4),
            health_bundle: HealthBundle::default(),
            game_mode: GameMode::default(),
        }
    }
}

impl From<PlayerSave> for PlayerBundle {
    fn from(save: PlayerSave) -> Self {
        PlayerBundle {
            transform: Transform::from_translation(save.position),
            camera: Camera::new(Transform {
                translation: save.camera_position,
                rotation: save.camera_rotation,
                ..default()
            }),
            inventory: save.inventory,
            equipment: save.equipment,
            health_bundle: HealthBundle::from_health(save.health),
            game_mode: save.game_mode,
            ..default()
        }
    }
}

// TODO: Remember equipped and send to player
//
/// The format the player is saved as in the database.
#[derive(Serialize, Deserialize)]
pub struct PlayerSave {
    position: DVec3,
    camera_position: DVec3,
    camera_rotation: DQuat,
    inventory: Inventory,
    equipment: Equipment,
    health: Health,
    game_mode: GameMode,
}

impl PlayerSave {
    fn save(&self, username: &str, database: &Database) {
        let conn = database.get_write_connection();

        let mut stmt = conn
            .prepare("INSERT OR REPLACE INTO players VALUES (?,?)")
            .unwrap();
        let json = serde_json::to_string(self).unwrap();

        stmt.execute(rusqlite::params![username, json]).unwrap();
    }

    fn load(username: &str, database: &Database) -> Option<Self> {
        let conn = database.get_read_connection();

        let mut stmt = conn
            .prepare("SELECT save FROM players WHERE name = ?")
            .unwrap();
        let mut rows = if let Ok(rows) = stmt.query([username]) {
            rows
        } else {
            return None;
        };

        // TODO: I've forgot how you're supposed to do this correctly
        if let Some(row) = rows.next().unwrap() {
            let json: String = row.get_unwrap(0);
            let save: PlayerSave = serde_json::from_str(&json).unwrap();
            return Some(save);
        } else {
            return None;
        };
    }
}

fn add_players(
    mut commands: Commands,
    net: Res<Server>,
    settings: Res<Settings>,
    database: Res<Database>,
    models: Res<Models>,
    mut respawn_events: EventWriter<RespawnEvent>,
    mut registration_events: EventWriter<RegisterInterfaceNode>,
    added_players: Query<(Entity, &Player), Added<Player>>,
) {
    for (player_entity, player) in added_players.iter() {
        let bundle = if let Some(save) = PlayerSave::load(&player.username, &database) {
            PlayerBundle::from(save)
        } else {
            respawn_events.write(RespawnEvent { player_entity });
            PlayerBundle::new(settings.game_mode)
        };

        net.send_one(
            player_entity,
            messages::Plugin::Enable("movement".to_owned()),
        );

        net.send_one(
            player_entity,
            messages::PlayerPosition {
                position: bundle.transform.translation,
            },
        );

        net.send_one(
            player_entity,
            messages::PlayerCameraPosition {
                position: bundle.camera.translation.as_vec3(),
            },
        );

        net.send_one(
            player_entity,
            messages::PlayerCameraRotation {
                rotation: bundle.camera.rotation.as_quat(),
            },
        );

        let model = models.get_by_name("player");

        let mut animation_player = AnimationPlayer::default();
        animation_player.set_move_animation(Some(model.animations["walk"]));
        animation_player.set_idle_animation(Some(model.animations["idle"]));
        animation_player.set_transition_time(0.15);

        let model_entity = commands
            .spawn(Model::Asset(model.id))
            .insert(ChildOf(player_entity))
            .id();
        animation_player.set_target(model_entity);

        let discard_items_entity = commands.spawn(DiscardItems).id();
        registration_events.write(RegisterInterfaceNode {
            player_entity,
            node_path: "".to_owned(),
            node_entity: discard_items_entity,
        });

        commands
            .entity(player_entity)
            .insert((bundle, animation_player))
            .add_child(discard_items_entity);
    }
}

#[derive(QueryData)]
struct PlayerQuery {
    player: &'static Player,
    transform: &'static Transform,
    camera: &'static Camera,
    inventory: &'static Inventory,
    equipment: &'static Equipment,
    health: &'static Health,
    game_mode: &'static GameMode,
}

fn save_player_data_on_disconnect(
    database: Res<Database>,
    mut network_events: EventReader<NetworkEvent>,
    mut players: Query<PlayerQuery>,
) {
    for network_event in network_events.read() {
        let NetworkEvent::Disconnected { entity } = network_event else {
            continue;
        };

        let Ok(player_query) = players.get_mut(*entity) else {
            continue;
        };

        PlayerSave {
            position: player_query.transform.translation,
            camera_position: player_query.camera.translation,
            camera_rotation: player_query.camera.rotation,
            inventory: player_query.inventory.clone(),
            equipment: player_query.equipment.clone(),
            health: player_query.health.clone(),
            game_mode: *player_query.game_mode,
        }
        .save(&player_query.player.username, &database);
    }
}

fn save_player_data_on_shutdown(
    database: Res<Database>,
    mut network_events: EventReader<NetworkEvent>,
    mut players: Query<PlayerQuery>,
) {
    for player_query in players.iter() {
        PlayerSave {
            position: player_query.transform.translation,
            camera_position: player_query.camera.translation,
            camera_rotation: player_query.camera.rotation,
            inventory: player_query.inventory.clone(),
            equipment: player_query.equipment.clone(),
            health: player_query.health.clone(),
            game_mode: *player_query.game_mode,
        }
        .save(&player_query.player.username, &database);
    }
}

#[derive(Event)]
pub struct RespawnEvent {
    pub player_entity: Entity,
}

// TODO: If it can't find a valid spawn point it will just oscillate in an infinite loop between the
// air chunk above and the one it can't find anything in.
// TODO: This might take a really long time to compute because of the chunk loading, and should
// probably be done ahead of time through an async task. Idk if the spawn point should change
// between each spawn. A good idea if it's really hard to validate that the player won't suffocate
// infinitely.
fn respawn_players(
    net: Res<Server>,
    world_properties: Res<WorldProperties>,
    world_map: Res<WorldMap>,
    database: Res<Database>,
    mut player_query: Query<&mut Transform, With<Player>>,
    mut heal_events: EventWriter<HealEvent>,
    mut respawn_events: EventReader<RespawnEvent>,
) {
    for respawn_event in respawn_events.read() {
        let blocks = Blocks::get();
        let air = blocks.get_id("air");

        let mut chunk_position = ChunkPosition::from(world_properties.spawn_point.center);
        let spawn_position = 'outer: loop {
            let chunk = futures_lite::future::block_on(Chunk::load(
                chunk_position,
                world_map.terrain_generator.clone(),
                database.clone(),
            ))
            .1;

            if chunk.is_uniform() && chunk[0] == air {
                break BlockPosition::from(chunk_position);
            }

            // Find two consecutive air blocks to spawn in
            for (i, block_column) in chunk.blocks.chunks_exact(Chunk::SIZE).enumerate() {
                let mut count = 0;
                for (j, block) in block_column.iter().enumerate() {
                    if count == 0 && *block == air {
                        count += 1;
                    } else if count == 1 && *block == air {
                        let mut spawn_position = BlockPosition::from(chunk_position)
                            + BlockPosition::from(i * Chunk::SIZE + j);
                        spawn_position.y -= 1;
                        break 'outer spawn_position;
                    } else {
                        count = 0;
                    }
                }
            }

            chunk_position.y += Chunk::SIZE as i32;
        };

        let spawn_position = spawn_position.as_dvec3() + DVec3::new(0.5, 0.0, 0.5);

        // TODO: Because of the latency before the client reports back its new position, the player will
        // be alive for a small moment at the spot they died, picking up their items again. So we
        // have to set the position server side too.
        let mut player_transform = player_query.get_mut(respawn_event.player_entity).unwrap();
        player_transform.translation = spawn_position;

        heal_events.write(HealEvent {
            player_entity: respawn_event.player_entity,
            healing: u32::MAX,
        });

        net.send_one(
            respawn_event.player_entity,
            messages::PlayerPosition {
                position: spawn_position,
            },
        );
    }
}

// TODO: This rotates the main player transform and lets propagation take care of the model.
// Propagation takes a long time to be sent to the clients because of unfortunate system ordering.
// This needs to be fixed on its own, but it will also become necessary to handle the player's
// models directly, as there will be a small collection of them.
fn rotate_player_model(
    mut player_query: Query<&mut Transform, With<Player>>,
    mut camera_rotation_events: EventReader<NetworkMessage<messages::PlayerCameraRotation>>,
) {
    for rotation_update in camera_rotation_events.read() {
        let mut transform = player_query.get_mut(rotation_update.player_entity).unwrap();

        let rotation = rotation_update.rotation.as_dquat();

        let theta = rotation.y.atan2(rotation.w);
        transform.rotation = DQuat::from_xyzw(0.0, theta.sin(), 0.0, theta.cos());
    }
}

fn on_gamemode_update(
    net: Res<Server>,
    player_query: Query<(Entity, &GameMode), Changed<GameMode>>,
) {
    for (player_entity, gamemode) in player_query.iter() {
        match gamemode {
            GameMode::Survival => {
                let mut health_visibility = messages::InterfaceNodeVisibilityUpdate::default();
                health_visibility.set_visible("health".to_owned());
                net.send_one(player_entity, health_visibility);

                let hotbar_visibility = messages::InterfaceVisibilityUpdate {
                    interface_path: "hotbar".to_owned(),
                    visible: true,
                };
                net.send_one(player_entity, hotbar_visibility);

                net.send_one(
                    player_entity,
                    messages::PluginData {
                        plugin: "movement".to_owned(),
                        data: bincode::serialize(&movement::MovementPluginPacket::GameMode(0))
                            .unwrap(),
                    },
                );

                // Change which mode is selected in the settings to reflect the server value
                net.send_one(
                    player_entity,
                    messages::GuiSetting::ButtonSelection {
                        name: "game_mode".to_owned(),
                        // Survival button index
                        selected: 0,
                    },
                );
            }
            GameMode::Creative => {
                let mut health_visibility = messages::InterfaceNodeVisibilityUpdate::default();
                health_visibility.set_hidden("health".to_owned());
                net.send_one(player_entity, health_visibility);

                let hotbar_visibility = messages::InterfaceVisibilityUpdate {
                    interface_path: "hotbar".to_owned(),
                    visible: true,
                };
                net.send_one(player_entity, hotbar_visibility);

                net.send_one(
                    player_entity,
                    messages::PluginData {
                        plugin: "movement".to_owned(),
                        data: bincode::serialize(&movement::MovementPluginPacket::GameMode(1))
                            .unwrap(),
                    },
                );

                net.send_one(
                    player_entity,
                    messages::GuiSetting::ButtonSelection {
                        name: "game_mode".to_owned(),
                        // Creative button index
                        selected: 1,
                    },
                );
            }
            GameMode::Spectator => {
                let mut health_visibility = messages::InterfaceNodeVisibilityUpdate::default();
                health_visibility.set_hidden("health".to_owned());
                net.send_one(player_entity, health_visibility);

                let hotbar_visibility = messages::InterfaceVisibilityUpdate {
                    interface_path: "hotbar".to_owned(),
                    visible: true,
                };
                net.send_one(player_entity, hotbar_visibility);

                net.send_one(
                    player_entity,
                    messages::PluginData {
                        plugin: "movement".to_owned(),
                        data: bincode::serialize(&movement::MovementPluginPacket::GameMode(2))
                            .unwrap(),
                    },
                );

                net.send_one(
                    player_entity,
                    messages::GuiSetting::ButtonSelection {
                        name: "game_mode".to_owned(),
                        // Spectator button index
                        selected: 2,
                    },
                );
            }
        }
    }
}

#[derive(Component)]
struct DiscardItems;

fn discard_items(
    mut commands: Commands,
    mut inventory_query: Query<(&mut HeldInterfaceStack, &GlobalTransform, &Camera), With<Player>>,
    mut interface_events: Query<
        (&mut InterfaceEvents, &ChildOf),
        (Changed<InterfaceEvents>, With<DiscardItems>),
    >,
) {
    for (mut interface_events, parent) in interface_events.iter_mut() {
        let (mut held_item, transform, camera) = inventory_query.get_mut(parent.0).unwrap();
        for event in interface_events.read() {
            if let messages::InterfaceInteraction::PlaceItem { quantity, .. } = *event {
                let discarded = held_item.take(quantity);
                if discarded.size() == 0 {
                    continue;
                }

                let dropped_item_position =
                    transform.translation() + camera.translation + camera.forward();
                commands.spawn((
                    DroppedItem::new(discarded),
                    Transform::from_translation(dropped_item_position),
                    Physics {
                        velocity: camera.forward() * 12.0,
                        ..default()
                    },
                ));
            }
        }
    }
}

fn handle_gui_settings(
    mut setting_events: EventReader<NetworkMessage<messages::GuiSetting>>,
    mut game_mode: Query<&mut GameMode>,
) {
    for setting in setting_events.read() {
        match &setting.message {
            messages::GuiSetting::ButtonSelection { name, selected } => match name.as_str() {
                "game_mode" => {
                    let mut game_mode = game_mode.get_mut(setting.player_entity).unwrap();
                    if *selected == 0 {
                        *game_mode = GameMode::Survival
                    } else if *selected == 1 {
                        *game_mode = GameMode::Creative
                    } else if *selected == 2 {
                        *game_mode = GameMode::Spectator
                    }
                }
                _ => (),
            },
            _ => (),
        }
    }
}
