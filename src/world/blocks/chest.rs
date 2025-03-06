use std::collections::{HashMap, HashSet};

use fmc::{
    bevy::ecs::system::EntityCommands,
    blocks::{BlockData, BlockPosition, Blocks},
    interfaces::{HeldInterfaceStack, InterfaceEvents, RegisterInterfaceNode},
    items::ItemStack,
    networking::Server,
    players::Player,
    prelude::*,
    protocol::messages,
    world::BlockUpdate,
};
use serde::{Deserialize, Serialize};

use crate::players::HandInteractions;

pub struct ChestPlugin;
impl Plugin for ChestPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ChestRegistry::default())
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (handle_block_hits, handle_interface_events, handle_despawn),
            );
    }
}

#[derive(Component, Serialize, Deserialize)]
struct Chest {
    inventory: Vec<ItemStack>,
}

impl Chest {
    fn new() -> Self {
        Self {
            inventory: vec![ItemStack::default(); 27],
        }
    }

    fn build_interface(&self) -> messages::InterfaceItemBoxUpdate {
        let mut item_box_update = messages::InterfaceItemBoxUpdate::default();
        for (i, item_stack) in self.inventory.iter().enumerate() {
            if !item_stack.is_empty() {
                item_box_update.add_itembox(
                    "chest",
                    i as u32,
                    item_stack.item().unwrap().id,
                    item_stack.size(),
                    None,
                    None,
                );
            } else {
                item_box_update.add_empty_itembox("chest", i as u32);
            }
        }

        item_box_update
    }
}

#[derive(Resource, Default)]
struct ChestRegistry {
    chest_to_players: HashMap<Entity, HashSet<Entity>>,
    player_to_chest: HashMap<Entity, Entity>,
}

impl ChestRegistry {
    fn remove_chest(&mut self, crafting_table_entity: Entity) {
        if let Some(player_entities) = self.chest_to_players.remove(&crafting_table_entity) {
            for entity in player_entities {
                self.player_to_chest.remove(&entity);
            }
        }
    }

    fn set_active_chest(&mut self, player_entity: Entity, crafting_table_entity: Entity) {
        if let Some(old_table_entity) = self.player_to_chest.remove(&player_entity) {
            self.chest_to_players
                .get_mut(&old_table_entity)
                .unwrap()
                .remove(&player_entity);
        }

        self.chest_to_players
            .entry(crafting_table_entity)
            .or_default()
            .insert(player_entity);
        self.player_to_chest
            .insert(player_entity, crafting_table_entity);
    }
}

fn setup(mut blocks: ResMut<Blocks>) {
    let block_id = blocks.get_id("chest");
    let block = blocks.get_config_mut(&block_id);
    block.set_spawn_function(spawn_function);
}

fn spawn_function(commands: &mut EntityCommands, block_data: Option<&BlockData>) {
    if let Some(block_data) = block_data {
        let chest: Chest = serde_json::from_slice(&block_data.0).unwrap();
        commands.insert(chest);
    } else {
        commands.insert(Chest::new());
    }

    commands.insert(HandInteractions::default());
}

fn handle_interface_events(
    net: Res<Server>,
    registry: Res<ChestRegistry>,
    mut player_query: Query<&mut HeldInterfaceStack, With<Player>>,
    mut input_events: Query<
        (Entity, &BlockPosition, &mut Chest, &mut InterfaceEvents),
        Changed<InterfaceEvents>,
    >,
    mut block_update_writer: EventWriter<BlockUpdate>,
) {
    for (chest_entity, block_position, mut chest, mut events) in input_events.iter_mut() {
        for event in events.read() {
            let mut held_item = player_query.get_mut(event.player_entity).unwrap();

            if let messages::InterfaceInteraction::TakeItem {
                quantity, index, ..
            } = *event
            {
                let Some(item_stack) = chest.inventory.get_mut(index as usize) else {
                    continue;
                };
                item_stack.transfer_to(&mut held_item, quantity);
            } else if let messages::InterfaceInteraction::PlaceItem {
                quantity, index, ..
            } = *event
            {
                let Some(item_stack) = chest.inventory.get_mut(index as usize) else {
                    continue;
                };
                held_item.transfer_to(item_stack, quantity);
            }

            block_update_writer.send(BlockUpdate::Data {
                position: *block_position,
                block_data: Some(serde_json::to_vec(&*chest).map(BlockData).unwrap()),
            });

            net.send_many(
                &registry.chest_to_players[&chest_entity],
                chest.build_interface(),
            );
        }
    }
}

fn handle_block_hits(
    net: Res<Server>,
    mut registry: ResMut<ChestRegistry>,
    mut block_hits: Query<(Entity, &Chest, &mut HandInteractions), Changed<HandInteractions>>,
    mut registration_events: EventWriter<RegisterInterfaceNode>,
) {
    for (chest_entity, chest, mut block_hits) in block_hits.iter_mut() {
        for player_entity in block_hits.read() {
            registry.set_active_chest(player_entity, chest_entity);

            registration_events.send(RegisterInterfaceNode {
                player_entity,
                node_path: "chest".to_owned(),
                node_entity: chest_entity,
            });

            net.send_one(player_entity, chest.build_interface());

            net.send_one(
                player_entity,
                messages::InterfaceVisibilityUpdate {
                    interface_path: "chest".to_owned(),
                    visible: true,
                },
            );
        }
    }
}

fn handle_despawn(
    mut registry: ResMut<ChestRegistry>,
    mut despawned_tables: RemovedComponents<Chest>,
) {
    for chest_entity in despawned_tables.read() {
        registry.remove_chest(chest_entity)
    }
}
