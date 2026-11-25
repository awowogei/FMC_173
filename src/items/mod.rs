use std::collections::{HashMap, HashSet};

use fmc::{
    assets::AssetSet,
    blocks::BlockId,
    database::Database,
    ecs::{lifecycle::HookContext, world::DeferredWorld},
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
        app.add_systems(PreStartup, load_items.after(AssetSet::Items))
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
        event_handlers: HashMap::new(),
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

/// Contains the names, ids, configs and event handlers of all the items in the game.
#[derive(Resource)]
pub struct Items {
    configs: HashMap<ItemId, ItemConfig>,
    // Map from filename/item name to item id.
    ids: HashMap<String, ItemId>,
    event_handlers: HashMap<ItemId, Entity>,
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

    pub fn get_event_handler(&self, item_id: ItemId) -> Option<Entity> {
        return self.event_handlers.get(&item_id).copied();
    }
}

/// Order systems that read [ItemEvents] after this [SystemSet] to minimize latency
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ItemSystems;

/// Each item that can be used must have a handler entity. When the item is used the player's
/// entity is pushed to the handler's [ItemEvents].
#[derive(Component)]
#[component(on_add = register_event_handler, on_remove = unregister_event_handler)]
pub struct ItemEvents {
    item_id: ItemId,
    player_entities: Vec<Entity>,
}

impl ItemEvents {
    pub fn new(item_id: ItemId) -> Self {
        Self {
            item_id,
            player_entities: Vec::new(),
        }
    }

    pub fn read(&mut self) -> impl Iterator<Item = Entity> + '_ {
        self.player_entities.drain(..)
    }

    pub fn push(&mut self, player_entity: Entity) {
        self.player_entities.push(player_entity);
    }
}

fn register_event_handler(mut world: DeferredWorld, context: HookContext) {
    let item_id = world.get::<ItemEvents>(context.entity).unwrap().item_id;
    let mut items = world.resource_mut::<Items>();
    if items.event_handlers.contains_key(&item_id) {
        panic!(
            "Item '{}' already has a handler",
            items.get_config(&item_id).name
        );
    }
    items.event_handlers.insert(item_id, context.entity);
}

fn unregister_event_handler(mut world: DeferredWorld, context: HookContext) {
    let item_id = world.get::<ItemEvents>(context.entity).unwrap().item_id;
    world
        .resource_mut::<Items>()
        .event_handlers
        .remove(&item_id);
}
