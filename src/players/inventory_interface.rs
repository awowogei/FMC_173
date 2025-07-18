use fmc::{
    interfaces::{
        HeldInterfaceStack, InterfaceEventRegistration, InterfaceEvents, RegisterInterfaceNode,
    },
    items::{ItemStack, Items},
    networking::{NetworkMessage, Server},
    players::Player,
    prelude::*,
    protocol::messages,
};

use crate::{
    items::crafting::{CraftingGrid, Recipes},
    players::{Equipment, Inventory},
};

pub struct InventoryInterfacePlugin;
impl Plugin for InventoryInterfacePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                initialize_interface,
                send_server_updates,
                (
                    handle_inventory_events,
                    handle_hotbar_events,
                    handle_equipment_events::<HelmetNode>,
                    handle_equipment_events::<ChestplateNode>,
                    handle_equipment_events::<LeggingsNode>,
                    handle_equipment_events::<BootsNode>,
                    handle_crafting_input_events,
                    handle_crafting_output_events,
                )
                    .after(InterfaceEventRegistration),
                equip_item,
            ),
        );
    }
}

fn initialize_interface(
    mut commands: Commands,
    net: Res<Server>,
    new_player_query: Query<Entity, Added<Player>>,
    mut registration_events: EventWriter<RegisterInterfaceNode>,
) {
    for player_entity in new_player_query.iter() {
        commands.entity(player_entity).with_children(|parent| {
            let inventory_entity = parent.spawn(InventoryNode).id();
            registration_events.write(RegisterInterfaceNode {
                player_entity,
                node_path: String::from("inventory"),
                node_entity: inventory_entity,
            });

            let hotbar_entity = parent.spawn(HotbarNode).id();
            registration_events.write(RegisterInterfaceNode {
                player_entity,
                node_path: String::from("hotbar"),
                node_entity: hotbar_entity,
            });

            let helmet_entity = parent.spawn(HelmetNode).id();
            registration_events.write(RegisterInterfaceNode {
                player_entity,
                node_path: String::from("equipment/helmet"),
                node_entity: helmet_entity,
            });

            let chestplate_entity = parent.spawn(ChestplateNode).id();
            registration_events.write(RegisterInterfaceNode {
                player_entity,
                node_path: String::from("equipment/chestplate"),
                node_entity: chestplate_entity,
            });

            let leggings_entity = parent.spawn(LeggingsNode).id();
            registration_events.write(RegisterInterfaceNode {
                player_entity,
                node_path: String::from("equipment/leggings"),
                node_entity: leggings_entity,
            });

            let boots_entity = parent.spawn(BootsNode).id();
            registration_events.write(RegisterInterfaceNode {
                player_entity,
                node_path: String::from("equipment/boots"),
                node_entity: boots_entity,
            });

            let crafting_input_entity = parent.spawn(CraftingInput).id();
            registration_events.write(RegisterInterfaceNode {
                player_entity,
                node_path: String::from("inventory/crafting_input"),
                node_entity: crafting_input_entity,
            });

            let crafting_output_entity = parent.spawn(CraftingOutput).id();
            registration_events.write(RegisterInterfaceNode {
                player_entity,
                node_path: String::from("inventory/crafting_output"),
                node_entity: crafting_output_entity,
            });
        });

        let mut crafting_items_boxes = messages::InterfaceItemBoxUpdate::default();
        for i in 0..4 {
            crafting_items_boxes.add_empty_itembox("inventory/crafting_input", i);
        }

        net.send_one(player_entity, crafting_items_boxes);
    }
}

fn send_server_updates(
    net: Res<Server>,
    inventory_query: Query<(Entity, &Inventory), Changed<Inventory>>,
    equipment_query: Query<(Entity, &Equipment), Changed<Equipment>>,
) {
    for (player_entity, inventory) in inventory_query.iter() {
        let mut inventory_node = messages::InterfaceItemBoxUpdate::default();

        for (i, item_stack) in inventory.iter().skip(9).enumerate() {
            if let Some(item) = item_stack.item() {
                inventory_node.add_itembox(
                    "inventory",
                    i as u32,
                    item.id,
                    item_stack.size(),
                    item.properties["durability"].as_u64().map(|v| v as u32),
                    item.properties["description"].as_str(),
                );
            } else {
                inventory_node.add_empty_itembox("inventory", i as u32);
                //inventory_node.add_itembox("inventory", i as u32, 1, 2, None, None);
            }
        }
        net.send_one(player_entity, inventory_node);

        let mut hotbar_node = messages::InterfaceItemBoxUpdate::default();

        for (i, item_stack) in inventory.iter().enumerate().take(9) {
            if let Some(item) = item_stack.item() {
                hotbar_node.add_itembox(
                    "hotbar",
                    i as u32,
                    item.id,
                    item_stack.size(),
                    item.properties["durability"].as_u64().map(|v| v as u32),
                    item.properties["description"].as_str(),
                );
            } else {
                hotbar_node.add_empty_itembox("hotbar", i as u32);
                //hotbar_node.add_itembox("hotbar", i as u32, 1, 2, None, None);
                //inventory.add_itembox(
                //    1, i as u32, 1, 2, None,
                //    None,
                //);
            }
        }

        net.send_one(player_entity, hotbar_node);
    }

    for (player_entity, equipment) in equipment_query.iter() {
        let mut equipment_node = messages::InterfaceItemBoxUpdate::default();
        for (item_stack, interface_path) in [
            (&equipment.helmet, "equipment/helmet"),
            (&equipment.chestplate, "equipment/chestplate"),
            (&equipment.leggings, "equipment/leggings"),
            (&equipment.boots, "equipment/boots"),
        ] {
            if let Some(item) = item_stack.item() {
                equipment_node.add_itembox(
                    interface_path,
                    0,
                    item.id,
                    item_stack.size(),
                    item.properties["durability"].as_u64().map(|v| v as u32),
                    item.properties["description"].as_str(),
                );
            } else {
                equipment_node.add_empty_itembox(interface_path, 0);
            }
        }

        net.send_one(player_entity, equipment_node);
    }
}

#[derive(Component)]
struct InventoryNode;

fn handle_inventory_events(
    mut inventory_query: Query<(&mut Inventory, &mut HeldInterfaceStack), With<Player>>,
    mut interface_events: Query<
        (&mut InterfaceEvents, &ChildOf),
        (Changed<InterfaceEvents>, With<InventoryNode>),
    >,
) {
    for (mut events, parent) in interface_events.iter_mut() {
        let (mut inventory, mut held_item) = inventory_query.get_mut(parent.0).unwrap();
        let inventory = inventory.bypass_change_detection();

        for event in events.read() {
            match *event {
                messages::InterfaceInteraction::TakeItem {
                    index, quantity, ..
                } => {
                    let Some(item_stack) = inventory.get_mut(index as usize + 9) else {
                        continue;
                    };
                    item_stack.transfer_to(&mut held_item, quantity);
                }
                messages::InterfaceInteraction::PlaceItem {
                    index, quantity, ..
                } => {
                    let Some(item_stack) = inventory.get_mut(index as usize + 9) else {
                        continue;
                    };
                    held_item.transfer_to(item_stack, quantity);
                }
                _ => continue,
            }
        }
    }
}

#[derive(Component)]
struct HotbarNode;

fn handle_hotbar_events(
    mut inventory_query: Query<(&mut Inventory, &mut HeldInterfaceStack), With<Player>>,
    mut interface_events: Query<
        (&mut InterfaceEvents, &ChildOf),
        (Changed<InterfaceEvents>, With<HotbarNode>),
    >,
) {
    for (mut events, parent) in interface_events.iter_mut() {
        let (mut inventory, mut held_item) = inventory_query.get_mut(parent.0).unwrap();
        let inventory = inventory.bypass_change_detection();

        for event in events.read() {
            match *event {
                messages::InterfaceInteraction::TakeItem {
                    index, quantity, ..
                } => {
                    let Some(item_stack) = inventory.get_mut(index as usize) else {
                        continue;
                    };
                    item_stack.transfer_to(&mut held_item, quantity);
                }
                messages::InterfaceInteraction::PlaceItem {
                    index, quantity, ..
                } => {
                    let Some(item_stack) = inventory.get_mut(index as usize) else {
                        continue;
                    };
                    held_item.transfer_to(item_stack, quantity);
                }
                _ => continue,
            }
        }
    }
}

#[derive(Component)]
struct HelmetNode;

#[derive(Component)]
struct ChestplateNode;

#[derive(Component)]
struct LeggingsNode;

#[derive(Component)]
struct BootsNode;

trait EquipmentNode {
    const NAME: &'static str;

    fn get_item_stack(equipment: &mut Equipment) -> &mut ItemStack;
}

impl EquipmentNode for HelmetNode {
    const NAME: &'static str = "helmet";

    fn get_item_stack(equipment: &mut Equipment) -> &mut ItemStack {
        &mut equipment.helmet
    }
}

impl EquipmentNode for ChestplateNode {
    const NAME: &'static str = "chestplate";

    fn get_item_stack(equipment: &mut Equipment) -> &mut ItemStack {
        &mut equipment.chestplate
    }
}

impl EquipmentNode for LeggingsNode {
    const NAME: &'static str = "leggings";

    fn get_item_stack(equipment: &mut Equipment) -> &mut ItemStack {
        &mut equipment.leggings
    }
}

impl EquipmentNode for BootsNode {
    const NAME: &'static str = "boots";

    fn get_item_stack(equipment: &mut Equipment) -> &mut ItemStack {
        &mut equipment.boots
    }
}

fn handle_equipment_events<T: EquipmentNode + Component>(
    items: Res<Items>,
    mut inventory_query: Query<(&mut Equipment, &mut HeldInterfaceStack), With<Player>>,
    mut interface_events: Query<
        (&mut InterfaceEvents, &ChildOf),
        (Changed<InterfaceEvents>, With<T>),
    >,
) {
    for (mut events, parent) in interface_events.iter_mut() {
        let (mut equipment, mut held) = inventory_query.get_mut(parent.0).unwrap();

        let equipment_item = T::get_item_stack(&mut *equipment);

        for event in events.read() {
            match *event {
                messages::InterfaceInteraction::TakeItem { quantity, .. } => {
                    if !held.item_stack.is_empty() {
                        continue;
                    }
                    equipment_item.transfer_to(&mut held, quantity);
                }
                messages::InterfaceInteraction::PlaceItem { quantity, .. } => {
                    let Some(item) = held.item() else {
                        continue;
                    };
                    if !items.get_config(&item.id).categories.contains(T::NAME) {
                        continue;
                    };
                    held.transfer_to(equipment_item, quantity);
                }
                _ => continue,
            }
        }
    }
}

#[derive(Component)]
struct CraftingInput;

fn handle_crafting_input_events(
    net: Res<Server>,
    recipes: Res<Recipes>,
    mut inventory_query: Query<(Entity, &mut HeldInterfaceStack, &mut CraftingGrid), With<Player>>,
    mut interface_events: Query<
        (&mut InterfaceEvents, &ChildOf),
        (Changed<InterfaceEvents>, With<CraftingInput>),
    >,
) {
    for (mut events, parent) in interface_events.iter_mut() {
        let (player_entity, mut held_item, mut crafting_input) =
            inventory_query.get_mut(parent.0).unwrap();
        for event in events.read() {
            match *event {
                messages::InterfaceInteraction::TakeItem {
                    index, quantity, ..
                } => {
                    let Some(item_stack) = crafting_input.get_mut(index as usize) else {
                        continue;
                    };
                    item_stack.transfer_to(&mut held_item, quantity);
                }
                messages::InterfaceInteraction::PlaceItem {
                    index, quantity, ..
                } => {
                    let Some(item_stack) = crafting_input.get_mut(index as usize) else {
                        continue;
                    };
                    held_item.transfer_to(item_stack, quantity);
                }
                _ => continue,
            }

            let mut update = messages::InterfaceItemBoxUpdate::default();

            if let Some(output) = recipes.get("crafting").get_output(&crafting_input) {
                update.add_itembox(
                    "inventory/crafting_output",
                    0,
                    output.item().unwrap().id,
                    output.capacity(),
                    None,
                    None,
                );
            } else {
                update.add_empty_itembox("inventory/crafting_output", 0);
            }

            net.send_one(player_entity, update);
        }
    }
}

#[derive(Component)]
struct CraftingOutput;

fn handle_crafting_output_events(
    net: Res<Server>,
    recipes: Res<Recipes>,
    mut inventory_query: Query<(Entity, &mut CraftingGrid, &mut HeldInterfaceStack), With<Player>>,
    mut interface_events: Query<
        (&mut InterfaceEvents, &ChildOf),
        (Changed<InterfaceEvents>, With<CraftingOutput>),
    >,
) {
    for (mut events, parent) in interface_events.iter_mut() {
        for event in events.read() {
            let messages::InterfaceInteraction::TakeItem { quantity, .. } = *event else {
                continue;
            };
            let (player_entity, mut crafting_input, mut held_item) =
                inventory_query.get_mut(parent.0).unwrap();
            let Some(output) = recipes.get("crafting").get_output(&crafting_input) else {
                continue;
            };

            if held_item.is_empty() || held_item.item() == output.item() {
                let amount = if held_item.is_empty() {
                    quantity
                } else {
                    std::cmp::min(held_item.remaining_capacity(), quantity)
                };

                if let Some(mut item_stack) =
                    recipes.get("crafting").craft(&mut crafting_input, amount)
                {
                    item_stack.transfer_to(&mut held_item, u32::MAX);
                } else {
                    continue;
                }

                let mut crafting_interface = messages::InterfaceItemBoxUpdate::default();

                for (i, item_stack) in crafting_input.iter().enumerate() {
                    if let Some(item) = item_stack.item() {
                        crafting_interface.add_itembox(
                            "inventory/crafting_input",
                            i as u32,
                            item.id,
                            item_stack.size(),
                            item.properties["durability"].as_u64().map(|v| v as u32),
                            item.properties["description"].as_str(),
                        );
                    } else {
                        crafting_interface.add_empty_itembox("inventory/crafting_input", i as u32);
                    }
                }

                if let Some(output) = recipes.get("crafting").get_output(&crafting_input) {
                    crafting_interface.add_itembox(
                        "inventory/crafting_output",
                        0,
                        output.item().unwrap().id,
                        output.capacity(),
                        None,
                        None,
                    );
                } else {
                    crafting_interface.add_empty_itembox("inventory/crafting_output", 0);
                }

                net.send_one(player_entity, crafting_interface)
            }
        }
    }
}

fn equip_item(
    net: Res<Server>,
    mut equip_events: EventReader<NetworkMessage<messages::InterfaceEquipItem>>,
    mut inventory: Query<&mut Inventory>,
) {
    for equip_event in equip_events.read() {
        if equip_event.interface_path != "hotbar" {
            return;
        }

        if equip_event.index > 8 {
            net.disconnect(equip_event.player_entity);
            continue;
        }

        let mut inventory = inventory.get_mut(equip_event.player_entity).unwrap();
        inventory.equipped_item = equip_event.index as usize;
    }
}
