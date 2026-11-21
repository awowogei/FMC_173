use std::collections::{HashMap, HashSet};

use fmc::{
    assets::AssetSet,
    blocks::BlockId,
    database::Database,
    items::{ITEM_CONFIG_PATH, ItemConfigEvent, ItemId},
    models::{ModelAssetId, Models},
    prelude::*,
};
use serde::Deserialize;

mod commands;
pub mod crafting;
mod dropped_items;

pub mod arrows;
pub mod bread;
pub mod bucket;
pub mod flint_and_steel;
pub mod hoes;
pub mod seeds;
pub mod spawn_crates;

pub use dropped_items::DroppedItem;

pub struct ItemPlugin;
impl Plugin for ItemPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ItemRegistry::default())
            .add_systems(PreStartup, load_items.after(AssetSet::Items))
            .add_plugins(commands::ItemCommandsPlugin)
            .add_plugins(dropped_items::DroppedItemsPlugin)
            .add_plugins(crafting::CraftingPlugin)
            .add_plugins(hoes::HoePlugin)
            .add_plugins(bread::BreadPlugin)
            .add_plugins(bucket::BucketPlugin)
            .add_plugins(flint_and_steel::FlintAndSteelPlugin)
            .add_plugins(spawn_crates::CratePlugin)
            .add_plugins(arrows::ArrowPlugin)
            .add_plugins(seeds::SeedPlugin);
    }
}

pub(crate) fn load_items(
    mut commands: Commands,
    models: Res<Models>,
    database: Res<Database>,
    mut item_config_events: MessageReader<ItemConfigEvent>,
) {
    let block_ids = database.load_block_ids();

    let mut items = Items {
        configs: HashMap::new(),
        ids: HashMap::new(),
    };

    for event in item_config_events.read() {
        let file_path = ITEM_CONFIG_PATH.to_owned() + &event.name + ".json";

        let json = match ItemConfigJson::deserialize(&event.json) {
            Ok(c) => c,
            Err(e) => panic!(
                "Couldn't read item config from '{}'\nError: {}",
                &file_path, e
            ),
        };

        let block = if let Some(block) = json.block {
            match block_ids.get(&block) {
                Some(block_id) => Some(*block_id),
                None => panic!(
                    "Failed to parse item config at: {}\nError: Missing block by the name: {}",
                    &file_path, &block
                ),
            }
        } else {
            None
        };

        let model_id = match models.get_config_by_name(&json.model) {
            Some(m) => m.id,
            None => {
                panic!(
                    "Failed to parse item config at: {}\nError: Missing model by the name: {}",
                    &file_path, &json.model
                );
            }
        };

        items.ids.insert(event.name.clone(), event.item_id);
        items.configs.insert(
            event.item_id,
            ItemConfig {
                id: event.item_id,
                name: json.name,
                block,
                model_id,
                categories: json.categories,
                tool: json.tool,
                damage: json.damage,
                fuel: json.fuel,
            },
        );
    }

    commands.insert_resource(items);
}

pub struct ItemConfig {
    /// The id of the item's ItemConfig
    pub id: ItemId,
    /// Name shown in interfaces
    pub name: String,
    /// Block placed by the item
    pub block: Option<BlockId>,
    /// Model used to render the item
    pub model_id: ModelAssetId,
    /// Names used to categorize the item, e.g "helmet". Used to restrict item placement in
    /// interfaces.
    pub categories: HashSet<String>,
    /// Present if the item is used as a tool to break blocks
    pub tool: Option<Tool>,
    pub damage: Option<u32>,
    pub fuel: Option<f32>,
}

#[derive(Deserialize)]
pub struct Tool {
    pub name: String,
    pub efficiency: f32,
}

#[derive(Deserialize)]
struct ItemConfigJson {
    name: String,
    /// Block name of the block this item can place.
    block: Option<String>,
    /// Item model filename
    model: String,
    #[serde(default)]
    categories: HashSet<String>,
    tool: Option<Tool>,
    damage: Option<u32>,
    fuel: Option<f32>,
}

/// Names and configs of all the items in the game.
#[derive(Resource)]
pub struct Items {
    configs: HashMap<ItemId, ItemConfig>,
    // Map from filename/item name to item id.
    ids: HashMap<String, ItemId>,
}

impl Items {
    #[track_caller]
    pub fn get_config(&self, item_id: &ItemId) -> &ItemConfig {
        return self.configs.get(item_id).unwrap();
    }

    pub fn get_config_by_name(&self, item_name: &str) -> Option<&ItemConfig> {
        let id = self.ids.get(item_name)?;
        return self.configs.get(id);
    }

    pub fn get_id(&self, name: &str) -> Option<ItemId> {
        return self.ids.get(name).cloned();
    }

    pub fn ids(&self) -> &HashMap<String, ItemId> {
        return &self.ids;
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

// TODO: This is such an ugly way to do it, same with how blocks do interactions too.
// Surely there must be a better way...
//
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
