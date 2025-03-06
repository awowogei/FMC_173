use std::collections::{HashMap, HashSet};

use fmc::{
    bevy::ecs::system::EntityCommands,
    blocks::{BlockData, BlockPosition, Blocks},
    interfaces::{HeldInterfaceStack, InterfaceEvents, RegisterInterfaceNode},
    items::{ItemStack, Items},
    networking::Server,
    players::Player,
    prelude::*,
    protocol::messages,
    world::{BlockUpdate, WorldMap},
};
use serde::{Deserialize, Serialize};

use crate::{
    items::crafting::{CraftingGrid, Recipes},
    players::HandInteractions,
};

pub struct FurnacePlugin;
impl Plugin for FurnacePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(FurnaceRegistry::default())
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (
                    handle_block_hits,
                    furnace,
                    handle_interface_events,
                    handle_despawn,
                ),
            );
    }
}

#[derive(Component, Serialize, Deserialize)]
struct Furnace {
    crucible: CraftingGrid,
    fuel: ItemStack,
    output: ItemStack,
    progress: Option<f32>,
    heat: f32,
    heat_max: f32,
    on: bool,
}

impl Furnace {
    fn new() -> Self {
        Self {
            crucible: CraftingGrid::with_size(1),
            fuel: ItemStack::default(),
            output: ItemStack::default(),
            progress: None,
            heat: 0.0,
            heat_max: 0.0,
            on: false,
        }
    }

    fn is_smelting(&self) -> bool {
        self.progress.is_some() && self.heat != 0.0
    }

    fn cold_start(&mut self, items: &Items, recipes: &Recipes) -> bool {
        if recipes
            .get("smelting")
            .get_output(&mut self.crucible)
            .is_some()
        {
            self.progress.get_or_insert(0.0);
        } else {
            self.progress = None;
            return false;
        }

        if self.heat == 0.0 {
            if let Some(item) = self.fuel.item() {
                let config = items.get_config(&item.id);
                if let Some(fuel) = config.properties.get("fuel") {
                    // TODO: Can't panic at runtime like this, make config proxy to deserialize
                    // these extra fields.
                    let fuel = fuel.as_f64().expect("The fuel property must a float") as f32;
                    self.fuel.take(1);
                    self.heat = fuel;
                    self.heat_max = fuel;
                    return true;
                }
            }
        }

        return false;
    }

    fn heat_stage(&self) -> usize {
        (self.heat / self.heat_max * HEAT_STAGES).ceil() as usize
    }

    fn progress_stage(&self) -> usize {
        let Some(progress) = self.progress else {
            return 0;
        };

        (progress / SMELT_TIME * PROGRESS_STAGES).floor() as usize
    }

    fn build_heat_interface(&self) -> messages::InterfaceNodeVisibilityUpdate {
        let mut node_update = messages::InterfaceNodeVisibilityUpdate::default();
        let heat_stage = self.heat_stage();

        for stage in 0..heat_stage {
            node_update.set_visible(format!("furnace/fire/{}", stage));
        }

        for stage in heat_stage..HEAT_STAGES as usize {
            node_update.set_hidden(format!("furnace/fire/{}", stage));
        }

        node_update
    }

    fn build_progress_interface(&self) -> messages::InterfaceNodeVisibilityUpdate {
        let mut node_update = messages::InterfaceNodeVisibilityUpdate::default();
        let progress_stage = self.progress_stage();

        for stage in 0..progress_stage {
            node_update.set_visible(format!("furnace/progress/{}", stage));
        }

        for stage in progress_stage..PROGRESS_STAGES as usize {
            node_update.set_hidden(format!("furnace/progress/{}", stage));
        }

        node_update
    }

    fn build_item_box_interface(&self) -> messages::InterfaceItemBoxUpdate {
        let mut item_box_update = messages::InterfaceItemBoxUpdate::default();
        for (item_stack, path) in [
            (&self.crucible[0], "furnace/crucible"),
            (&self.fuel, "furnace/fuel"),
            (&self.output, "furnace/output"),
        ] {
            if !item_stack.is_empty() {
                item_box_update.add_itembox(
                    path,
                    0,
                    item_stack.item().unwrap().id,
                    item_stack.size(),
                    None,
                    None,
                );
            } else {
                item_box_update.add_empty_itembox(path, 0);
            }
        }

        item_box_update
    }
}

#[derive(Resource, Default)]
struct FurnaceRegistry {
    furnace_to_players: HashMap<Entity, HashSet<Entity>>,
    player_to_furnace: HashMap<Entity, Entity>,
}

impl FurnaceRegistry {
    fn remove_furnace(&mut self, furnace_entity: Entity) {
        if let Some(player_entities) = self.furnace_to_players.remove(&furnace_entity) {
            for entity in player_entities {
                self.player_to_furnace.remove(&entity);
            }
        }
    }

    fn set_active_furnace(&mut self, player_entity: Entity, furnace_entity: Entity) {
        if let Some(old_table_entity) = self.player_to_furnace.remove(&player_entity) {
            self.furnace_to_players
                .get_mut(&old_table_entity)
                .unwrap()
                .remove(&player_entity);
        }

        self.furnace_to_players
            .entry(furnace_entity)
            .or_default()
            .insert(player_entity);
        self.player_to_furnace.insert(player_entity, furnace_entity);
    }
}

fn setup(mut blocks: ResMut<Blocks>) {
    let block_id = blocks.get_id("furnace");
    let block = blocks.get_config_mut(&block_id);
    block.set_spawn_function(spawn_function);

    let block_id = blocks.get_id("furnace_on");
    let block = blocks.get_config_mut(&block_id);
    block.set_spawn_function(spawn_function);
}

fn spawn_function(commands: &mut EntityCommands, block_data: Option<&BlockData>) {
    if let Some(block_data) = block_data {
        let furnace: Furnace = bincode::deserialize(&*block_data).unwrap();
        commands.insert(furnace);
    } else {
        commands.insert(Furnace::new());
    }

    commands.insert(HandInteractions::default());
}

const HEAT_STAGES: f32 = 12.0;
const PROGRESS_STAGES: f32 = 16.0;
const SMELT_TIME: f32 = 10.0;

fn furnace(
    net: Res<Server>,
    world_map: Res<WorldMap>,
    time: Res<Time>,
    recipes: Res<Recipes>,
    items: Res<Items>,
    registry: Res<FurnaceRegistry>,
    mut furnaces_query: Query<(Entity, &BlockPosition, &mut Furnace)>,
    mut block_update_writer: EventWriter<BlockUpdate>,
) {
    for (entity, block_position, mut furnace) in furnaces_query.iter_mut() {
        let prev_heat = furnace.heat_stage();
        furnace.heat = (furnace.heat - time.delta_secs()).max(0.0);

        // If the furnace wasn't cold, but is now, try to fuel it
        if furnace.heat == 0.0 && prev_heat != 0 {
            furnace.cold_start(&items, &recipes);
        }

        if prev_heat != furnace.heat_stage() {
            if let Some(players) = registry.furnace_to_players.get(&entity) {
                net.send_many(players, furnace.build_heat_interface());
            }
        }

        if furnace.heat != 0.0 && !furnace.on {
            block_update_writer.send(BlockUpdate::Swap {
                position: *block_position,
                block_id: Blocks::get().get_id("furnace_on"),
                block_state: world_map.get_block_state(*block_position),
            });

            furnace.on = true;
        } else if furnace.heat == 0.0 && furnace.on {
            block_update_writer.send(BlockUpdate::Swap {
                position: *block_position,
                block_id: Blocks::get().get_id("furnace"),
                block_state: world_map.get_block_state(*block_position),
            });

            furnace.on = false;
        }

        if furnace.is_smelting() {
            let prev_progress = furnace.progress_stage();
            let progress = furnace.progress.unwrap();
            furnace.progress = Some(progress + time.delta_secs());

            if prev_progress != furnace.progress_stage() {
                if let Some(players) = registry.furnace_to_players.get(&entity) {
                    net.send_many(players, furnace.build_progress_interface());
                }
            }

            if progress >= SMELT_TIME {
                let smelting = recipes.get("smelting");
                if let Some(mut output) = smelting.craft(&mut furnace.crucible, 1) {
                    output.transfer_to(&mut furnace.output, u32::MAX);
                    // Furnaces can store an unlimited amount of items in its output
                    furnace.output.set_capacity(u32::MAX);
                }

                furnace.progress = None;

                furnace.cold_start(&items, &recipes);

                if let Some(players) = registry.furnace_to_players.get(&entity) {
                    net.send_many(players, furnace.build_item_box_interface());
                    net.send_many(players, furnace.build_progress_interface());
                }
            }
        }
    }
}

fn handle_interface_events(
    net: Res<Server>,
    registry: Res<FurnaceRegistry>,
    items: Res<Items>,
    recipes: Res<Recipes>,
    mut player_query: Query<&mut HeldInterfaceStack, With<Player>>,
    mut input_events: Query<(Entity, &mut Furnace, &mut InterfaceEvents), Changed<InterfaceEvents>>,
) {
    for (furnace_entity, mut furnace, mut events) in input_events.iter_mut() {
        for event in events.read() {
            let mut held_item = player_query.get_mut(event.player_entity).unwrap();

            if let messages::InterfaceInteraction::TakeItem {
                interface_path,
                quantity,
                ..
            } = &*event
            {
                if interface_path.ends_with("crucible") {
                    furnace.crucible[0].transfer_to(&mut held_item, *quantity);
                } else if interface_path.ends_with("fuel") {
                    furnace.fuel.transfer_to(&mut held_item, *quantity);
                } else if interface_path.ends_with("output") {
                    furnace.output.transfer_to(&mut held_item, *quantity);
                }
            } else if let messages::InterfaceInteraction::PlaceItem {
                interface_path,
                quantity,
                ..
            } = &*event
            {
                if interface_path.ends_with("crucible") {
                    held_item.transfer_to(&mut furnace.crucible[0], *quantity);
                } else if interface_path.ends_with("fuel") {
                    held_item.transfer_to(&mut furnace.fuel, *quantity);
                }
            }

            furnace.cold_start(&items, &recipes);
            net.send_many(
                &registry.furnace_to_players[&furnace_entity],
                furnace.build_heat_interface(),
            );
            net.send_many(
                &registry.furnace_to_players[&furnace_entity],
                furnace.build_progress_interface(),
            );
            net.send_many(
                &registry.furnace_to_players[&furnace_entity],
                furnace.build_item_box_interface(),
            );
        }
    }
}

fn handle_block_hits(
    net: Res<Server>,
    mut registry: ResMut<FurnaceRegistry>,
    mut block_hits: Query<(Entity, &Furnace, &mut HandInteractions), Changed<HandInteractions>>,
    mut registration_events: EventWriter<RegisterInterfaceNode>,
) {
    for (furnace_entity, furnace, mut block_hits) in block_hits.iter_mut() {
        for player_entity in block_hits.read() {
            registry.set_active_furnace(player_entity, furnace_entity);

            registration_events.send(RegisterInterfaceNode {
                player_entity,
                node_path: String::from("furnace/crucible"),
                node_entity: furnace_entity,
            });
            registration_events.send(RegisterInterfaceNode {
                player_entity,
                node_path: String::from("furnace/fuel"),
                node_entity: furnace_entity,
            });
            registration_events.send(RegisterInterfaceNode {
                player_entity,
                node_path: String::from("furnace/output"),
                node_entity: furnace_entity,
            });

            net.send_one(player_entity, furnace.build_heat_interface());
            net.send_one(player_entity, furnace.build_progress_interface());
            net.send_one(player_entity, furnace.build_item_box_interface());

            net.send_one(
                player_entity,
                messages::InterfaceVisibilityUpdate {
                    interface_path: "furnace".to_owned(),
                    visible: true,
                },
            );
        }
    }
}

fn save_state(mut table_query: Query<(&Furnace, &mut BlockData), Changed<Furnace>>) {
    for (furnace, mut block_data) in table_query.iter_mut() {
        *block_data = bincode::serialize(furnace).map(BlockData).unwrap();
    }
}

fn handle_despawn(
    mut registry: ResMut<FurnaceRegistry>,
    mut despawned_tables: RemovedComponents<Furnace>,
) {
    for furnace_entity in despawned_tables.read() {
        registry.remove_furnace(furnace_entity)
    }
}
