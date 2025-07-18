use std::collections::{HashMap, HashSet};

use fmc::{
    bevy::ecs::system::EntityCommands,
    blocks::{BlockData, Blocks},
    interfaces::{HeldInterfaceStack, InterfaceEvents, RegisterInterfaceNode},
    items::{ItemStack, Items},
    networking::Server,
    players::Player,
    prelude::*,
    protocol::messages,
};
use serde::{Deserialize, Serialize};

use crate::{
    items::crafting::{CraftingGrid, Recipes},
    players::HandInteractions,
};

pub struct CraftingTablePlugin;
impl Plugin for CraftingTablePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CraftingTableRegistry::default())
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (handle_block_hits, handle_interface_events, handle_despawn),
            );
    }
}

#[derive(Component, Deref, DerefMut, Serialize, Deserialize)]
struct CraftingTable(CraftingGrid);

impl CraftingTable {
    fn build_input_interface(&self, interface_update: &mut messages::InterfaceItemBoxUpdate) {
        for (i, item_stack) in self.iter().enumerate() {
            if let Some(item) = item_stack.item() {
                interface_update.add_itembox(
                    "crafting_table/input",
                    i as u32,
                    item.id,
                    item_stack.size(),
                    None,
                    None,
                );
            } else {
                interface_update.add_empty_itembox("crafting_table/input", i as u32);
            }
        }
    }

    fn build_output_interface(
        &self,
        recipes: &Recipes,
        interface_update: &mut messages::InterfaceItemBoxUpdate,
    ) {
        if let Some(output) = recipes.get("crafting").get_output(self) {
            interface_update.add_itembox(
                "crafting_table/output",
                0,
                output.item().unwrap().id,
                output.capacity(),
                None,
                None,
            );
        } else {
            interface_update.add_empty_itembox("crafting_table/output", 0);
        }
    }
}

#[derive(Resource, Default)]
struct CraftingTableRegistry {
    table_to_players: HashMap<Entity, HashSet<Entity>>,
    player_to_table: HashMap<Entity, Entity>,
}

impl CraftingTableRegistry {
    fn remove_table(&mut self, crafting_table_entity: Entity) {
        if let Some(player_entities) = self.table_to_players.remove(&crafting_table_entity) {
            for entity in player_entities {
                self.player_to_table.remove(&entity);
            }
        }
    }

    fn set_active_table(&mut self, player_entity: Entity, crafting_table_entity: Entity) {
        if let Some(old_table_entity) = self.player_to_table.remove(&player_entity) {
            self.table_to_players
                .get_mut(&old_table_entity)
                .unwrap()
                .remove(&player_entity);
        }

        self.table_to_players
            .entry(crafting_table_entity)
            .or_default()
            .insert(player_entity);
        self.player_to_table
            .insert(player_entity, crafting_table_entity);
    }
}

fn setup(mut blocks: ResMut<Blocks>) {
    let block_id = blocks.get_id("crafting_table");
    let block = blocks.get_config_mut(&block_id);
    block.set_spawn_function(spawn_function);
}

fn spawn_function(commands: &mut EntityCommands, block_data: Option<&BlockData>) {
    if let Some(block_data) = block_data {
        let crafting_table: CraftingTable = bincode::deserialize(&*block_data).unwrap();
        commands.insert(crafting_table);
    } else {
        commands.insert(CraftingTable(CraftingGrid::with_size(9)));
    }

    commands.insert(HandInteractions::default());
}

fn handle_interface_events(
    net: Res<Server>,
    registry: Res<CraftingTableRegistry>,
    recipes: Res<Recipes>,
    mut player_query: Query<&mut HeldInterfaceStack, With<Player>>,
    mut input_events: Query<
        (Entity, &mut CraftingTable, &mut InterfaceEvents),
        Changed<InterfaceEvents>,
    >,
) {
    for (crafting_table_entity, mut crafting_table, mut events) in input_events.iter_mut() {
        for event in events.read() {
            let mut held_item = player_query.get_mut(event.player_entity).unwrap();

            let mut interface_update = messages::InterfaceItemBoxUpdate::default();

            if let messages::InterfaceInteraction::TakeItem {
                interface_path,
                index,
                quantity,
            } = &*event
            {
                if interface_path.ends_with("input") {
                    let Some(item_stack) = crafting_table.get_mut(*index as usize) else {
                        continue;
                    };
                    item_stack.transfer_to(&mut held_item, *quantity);

                    crafting_table.build_output_interface(&recipes, &mut interface_update);
                } else if interface_path.ends_with("output") {
                    let Some(output) = recipes.get("crafting").get_output(&crafting_table) else {
                        continue;
                    };

                    if held_item.is_empty() || held_item.item() == output.item() {
                        let amount = if held_item.is_empty() {
                            *quantity
                        } else {
                            std::cmp::min(held_item.remaining_capacity(), *quantity)
                        };

                        if let Some(mut item_stack) =
                            recipes.get("crafting").craft(&mut crafting_table, amount)
                        {
                            item_stack.transfer_to(&mut held_item, u32::MAX);
                        } else {
                            continue;
                        }

                        crafting_table.build_input_interface(&mut interface_update);
                        crafting_table.build_output_interface(&recipes, &mut interface_update);
                    }
                }
            } else if let messages::InterfaceInteraction::PlaceItem {
                interface_path,
                index,
                quantity,
            } = &*event
            {
                if !interface_path.ends_with("input") {
                    continue;
                }

                let Some(item_stack) = crafting_table.get_mut(*index as usize) else {
                    continue;
                };
                held_item.transfer_to(item_stack, *quantity);

                crafting_table.build_output_interface(&recipes, &mut interface_update);
            }

            if !interface_update.updates.is_empty() {
                net.send_many(
                    &registry.table_to_players[&crafting_table_entity],
                    interface_update,
                );
            }
        }
    }
}

fn handle_block_hits(
    net: Res<Server>,
    mut registry: ResMut<CraftingTableRegistry>,
    recipes: Res<Recipes>,
    mut block_hits: Query<
        (Entity, &CraftingTable, &mut HandInteractions),
        Changed<HandInteractions>,
    >,
    mut registration_events: EventWriter<RegisterInterfaceNode>,
) {
    for (crafting_table_entity, crafting_table, mut block_hits) in block_hits.iter_mut() {
        for player_entity in block_hits.read() {
            registry.set_active_table(player_entity, crafting_table_entity);

            registration_events.write(RegisterInterfaceNode {
                player_entity,
                node_path: String::from("crafting_table/input"),
                node_entity: crafting_table_entity,
            });
            registration_events.write(RegisterInterfaceNode {
                player_entity,
                node_path: String::from("crafting_table/output"),
                node_entity: crafting_table_entity,
            });

            let mut itembox_update = messages::InterfaceItemBoxUpdate::default();
            crafting_table.build_input_interface(&mut itembox_update);
            crafting_table.build_output_interface(&recipes, &mut itembox_update);
            net.send_one(player_entity, itembox_update);

            net.send_one(
                player_entity,
                messages::InterfaceVisibilityUpdate {
                    interface_path: "crafting_table".to_owned(),
                    visible: true,
                },
            );
        }
    }
}

fn save_state(mut table_query: Query<(&CraftingTable, &mut BlockData), Changed<CraftingTable>>) {
    for (crafting_table, mut block_data) in table_query.iter_mut() {
        *block_data = bincode::serialize(crafting_table).map(BlockData).unwrap();
    }
}

fn handle_despawn(
    mut registry: ResMut<CraftingTableRegistry>,
    mut despawned_tables: RemovedComponents<CraftingTable>,
) {
    for crafting_table_entity in despawned_tables.read() {
        registry.remove_table(crafting_table_entity)
    }
}
