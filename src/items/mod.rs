use std::collections::HashMap;

use fmc::{items::ItemId, prelude::*};

pub mod crafting;
mod dropped_items;

pub mod bread;
pub mod hoes;
pub mod seeds;

pub use dropped_items::DroppedItem;

pub struct ItemPlugin;
impl Plugin for ItemPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ItemRegistry::default())
            .add_plugins(dropped_items::DroppedItemsPlugin)
            .add_plugins(crafting::CraftingPlugin)
            .add_plugins(hoes::HoePlugin)
            .add_plugins(bread::BreadPlugin)
            .add_plugins(seeds::SeedPlugin);
    }
}

/// Order systems that handle item uses after this [SystemSet] to minimize latency
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ItemUseSystems;

// TODO: Transfer this over to fmc lib and have the entity be part of the ItemConfig of the item
// that can be used?
//
/// Each item that can be used must have a handler entity registered here. When the item is used
/// the entity's [ItemUses] will be updated with the entity of the player that used it.
#[derive(Resource, Deref, DerefMut, Default)]
pub struct ItemRegistry(HashMap<ItemId, Entity>);

impl ItemRegistry {
    /// Register a new handler for an item
    pub fn insert(&mut self, item_id: ItemId, handler_entity: Entity) {
        self.0.insert(item_id, handler_entity);
    }
}

/// List of player entities that have used the item during the last tick.
///
/// Attach this to an entity and register the entity as a handler in the [ItemRegistry].
#[derive(Component, Default)]
pub struct ItemUses(Vec<Entity>);

impl ItemUses {
    pub fn read(&mut self) -> impl Iterator<Item = Entity> + '_ {
        self.0.drain(..)
    }

    pub fn push(&mut self, player_entity: Entity) {
        self.0.push(player_entity);
    }
}
