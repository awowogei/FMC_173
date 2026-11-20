use std::collections::{HashMap, HashSet};

use fmc::{
    assets::AssetSet,
    blocks::{
        BLOCK_CONFIG_PATH, BLOCK_MATERIAL_PATH, BlockConfigEvent, BlockFace, BlockId,
        BlockPlacement, BlockQuad,
    },
    items::{
        ItemStack,
        drop_table::{DropTable, DropTableJson},
    },
    math::DVec3,
    models::{ModelAssetId, Models},
    physics::{Collider, Friction},
    prelude::*,
    random::Rng,
};
use serde::Deserialize;

use crate::items::{ItemConfig, Items};

mod chest;
mod crafting_table;
mod door;
mod furnace;
pub mod tnt;
mod torch;
pub mod water;
mod wheat;

/// Adds systems for all blocks that are dynamic in some way
pub(super) struct BlocksPlugin;
impl Plugin for BlocksPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreStartup,
            load_blocks
                .after(AssetSet::Blocks)
                .after(crate::items::load_items),
        )
        .add_plugins(crafting_table::CraftingTablePlugin)
        .add_plugins(chest::ChestPlugin)
        .add_plugins(furnace::FurnacePlugin)
        .add_plugins(torch::TorchPlugin)
        .add_plugins(water::WaterPlugin)
        .add_plugins(door::DoorPlugin)
        .add_plugins(tnt::TntPlugin)
        .add_plugins(wheat::WheatPlugin);
    }
}

fn load_blocks(
    mut commands: Commands,
    models: Res<Models>,
    items: Res<Items>,
    mut block_config_events: MessageReader<BlockConfigEvent>,
) {
    let materials = load_block_materials();

    let mut ids = HashMap::new();
    let mut configs = Vec::new();
    configs.resize_with(block_config_events.len(), Option::default);

    for event in block_config_events.read() {
        let json = match BlockConfigJson::deserialize(&event.json) {
            Ok(c) => c,
            Err(e) => panic!(
                "Failed to read block config for '{}' under {}\nError: {}",
                &event.name, BLOCK_CONFIG_PATH, e
            ),
        };

        let model = if let Some(model_name) = &json.model {
            match models.get_config_by_name(model_name) {
                Some(m) => Some(m.id),
                None => panic!(
                    "Failed to read block config for '{}' under {}\nError: no model by the name: {}",
                    &event.name, BLOCK_CONFIG_PATH, model_name
                ),
            }
        } else {
            None
        };

        let collider = match Collider::from_block_json(&event.json, &models) {
            Ok(c) => c,
            Err(e) => panic!(
                "Failed to read block config for '{}' under {}\nError: {}",
                &event.name, BLOCK_CONFIG_PATH, e
            ),
        };

        let drop = json.drop.map(|drop| {
            let table = match &drop.output {
                BlockDropOutputJson::Item(name) => {
                    let Some(item) = items.get_id(name) else {
                        panic!(
                            "Failed to read 'drop' field for block '{}' under {}\nError: No item by the name {}",
                            &event.name, BLOCK_CONFIG_PATH, name
                        );
                    };
                    DropTable::new(1.0, &[(item, 1.0, 1, 1)]).unwrap()
                }
                BlockDropOutputJson::Table(table_json) => match DropTable::from_json(table_json) {
                    Ok(t) => t,
                    Err(e) => panic!(
                        "Failed to read 'drop' field for block '{}' under {}\nError: {}",
                        &event.name, BLOCK_CONFIG_PATH, e
                    ),
                },
            };
            BlockDrop {
                requires_tool: drop.requires_tool,
                table,
            }
        });

        let particle_textures = if let Some(particle_texture) = json.particle_texture {
            Some(BlockFaceTextures {
                top: particle_texture.clone(),
                bottom: particle_texture.clone(),
                right: particle_texture.clone(),
                left: particle_texture.clone(),
                front: particle_texture.clone(),
                back: particle_texture,
            })
        } else if let Some(faces) = json.faces {
            // The path must be relative to /textures/ but faces are specified relative to
            // /textures/blocks
            let path = "blocks/";
            Some(BlockFaceTextures {
                top: path.to_owned() + &faces.top,
                bottom: path.to_owned() + &faces.bottom,
                right: path.to_owned() + &faces.right,
                left: path.to_owned() + &faces.left,
                front: path.to_owned() + &faces.front,
                back: path.to_owned() + &faces.back,
            })
        } else {
            None
        };

        let particle_color = json
            .material
            .as_ref()
            .and_then(|material_name| materials.get(material_name))
            .map(|material| {
                Vec4::new(
                    material.base_color.red,
                    material.base_color.green,
                    material.base_color.blue,
                    material.base_color.alpha,
                )
            });

        ids.insert(event.name.clone(), event.block_id);
        configs[event.block_id as usize] = Some(BlockConfig {
            name: json.name,
            model,
            friction: json.friction.unwrap_or(Friction::Drag(DVec3::ZERO)),
            replaceable: json.replaceable,
            collider,
            quads: json.quads,
            placement: json.placement,
            hardness: json.hardness,
            blast_resistance: json.blast_resistance.unwrap_or(1.0),
            tools: json.tools,
            drop,
            sound: json.sound,
            particle_textures,
            particle_color,
        });
    }

    commands.insert_resource(Blocks {
        configs: configs.into_iter().flatten().collect(),
        ids,
    });
}

fn load_block_materials() -> HashMap<String, BlockMaterial> {
    let mut materials = HashMap::new();

    let directory = match std::fs::read_dir(BLOCK_MATERIAL_PATH) {
        Ok(d) => d,
        Err(e) => panic!(
            "Failed to read block materials from: '{}'\nError: {}",
            BLOCK_MATERIAL_PATH, e
        ),
    };

    for dir_entry in directory {
        let file_path = match dir_entry {
            Ok(p) => p.path(),
            Err(e) => panic!(
                "Failed to read block materials from: '{}'\nError: {}",
                BLOCK_MATERIAL_PATH, e
            ),
        };

        let material_name = file_path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();

        let contents = match std::fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(e) => panic!(
                "Failed to open block material config.\nPath: {}\nError: {}",
                file_path.display(),
                e
            ),
        };

        let material: BlockMaterial = match serde_json::from_str(&contents) {
            Ok(c) => c,
            Err(e) => panic!(
                "Failed to read material configuration at path: '{}'\nError: {}",
                file_path.display(),
                e
            ),
        };

        materials.insert(material_name, material);
    }

    return materials;
}

/// The configurations and ids of the blocks in the game.
#[derive(Resource)]
pub struct Blocks {
    // block id -> block config
    configs: Vec<BlockConfig>,
    // block name -> block id
    ids: HashMap<String, BlockId>,
}

impl Blocks {
    pub fn get_config(&self, block_id: &BlockId) -> &BlockConfig {
        return &self.configs[*block_id as usize];
    }

    #[track_caller]
    pub fn get_id(&self, block_name: &str) -> BlockId {
        match self.ids.get(block_name) {
            Some(b) => *b,
            None => panic!("No block with name '{}'", block_name),
        }
    }

    pub fn contains_block(&self, block_name: &str) -> bool {
        return self.ids.contains_key(block_name);
    }

    pub fn ids(&self) -> &HashMap<String, BlockId> {
        return &self.ids;
    }

    pub fn configs(&self) -> &Vec<BlockConfig> {
        return &self.configs;
    }
}

/// The configuration values of a block
pub struct BlockConfig {
    /// Name of the block
    pub name: String,
    /// If a model is used to represent this block, this contains its model id
    pub model: Option<ModelAssetId>,
    /// The friction of the block's surfaces or its drag.
    pub friction: Friction,
    /// Makes it possible to replace the block by placing another in its position.
    pub replaceable: bool,
    /// Collider used for physics and hit detection.
    pub collider: Collider,
    pub quads: Option<Vec<BlockQuad>>,
    /// Rules for how the block can be placed by the player.
    pub placement: BlockPlacement,
    /// How long it takes to break the block without a tool in seconds, None if the block should
    /// not be breakable. e.g. water, air
    pub hardness: Option<f32>,
    /// The block's resistance against explosions
    pub blast_resistance: f32,
    // Which tool categories will break this block faster.
    pub tools: HashSet<String>,
    // Which item(s) the block drops.
    drop: Option<BlockDrop>,
    /// Sound files associated with the block
    pub sound: Sounds,
    /// Texture path for each face used for particles when breaking blocks
    particle_textures: Option<BlockFaceTextures>,
    particle_color: Option<Vec4>,
}

impl BlockConfig {
    pub fn drag(&self) -> Option<DVec3> {
        self.friction.drag()
    }

    pub fn is_solid(&self) -> bool {
        matches!(self.friction, Friction::Surface { .. })
    }

    pub fn drop(&self, tool: Option<&ItemConfig>, rng: &mut Rng) -> Option<ItemStack> {
        let block_drop = self.drop.as_ref()?;

        if block_drop.requires_tool {
            let tool_name = &tool?.tool.as_ref()?.name;
            if !self.tools.contains(tool_name) {
                return None;
            }
        }

        block_drop.table.drop(rng)
    }

    pub fn particle_texture(&self, block_face: BlockFace) -> Option<&str> {
        if let Some(paths) = &self.particle_textures {
            let path = match block_face {
                BlockFace::Top => &paths.top,
                BlockFace::Bottom => &paths.bottom,
                BlockFace::Right => &paths.right,
                BlockFace::Left => &paths.left,
                BlockFace::Front => &paths.front,
                BlockFace::Back => &paths.back,
            };
            Some(path)
        } else {
            None
        }
    }

    pub fn particle_color(&self) -> Option<Vec4> {
        return self.particle_color;
    }
}

#[derive(Deserialize)]
struct BlockConfigJson {
    name: String,
    friction: Option<Friction>,
    #[serde(default)]
    replaceable: bool,
    model: Option<String>,
    quads: Option<Vec<BlockQuad>>,
    faces: Option<BlockFaceTextures>,
    #[serde(default)]
    placement: BlockPlacement,
    hardness: Option<f32>,
    blast_resistance: Option<f32>,
    #[serde(default)]
    tools: HashSet<String>,
    drop: Option<BlockDropJson>,
    material: Option<String>,
    particle_texture: Option<String>,
    #[serde(default)]
    sound: Sounds,
}

#[derive(Deserialize)]
struct BlockDropJson {
    #[serde(default)]
    requires_tool: bool,
    #[serde(flatten)]
    output: BlockDropOutputJson,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum BlockDropOutputJson {
    Item(String),
    Table(DropTableJson),
}

struct BlockDrop {
    requires_tool: bool,
    table: DropTable,
}

// Paths to textures used by a cube, relative to /textures/
#[derive(Deserialize)]
struct BlockFaceTextures {
    top: String,
    bottom: String,
    left: String,
    right: String,
    front: String,
    back: String,
}

/// Interaction sounds
#[derive(Deserialize, Default)]
pub struct Sounds {
    #[serde(default)]
    place: Vec<String>,
    #[serde(default)]
    step: Vec<String>,
    #[serde(default)]
    hit: Vec<String>,
    #[serde(default)]
    destroy: Vec<String>,
}

impl Sounds {
    pub fn step(&self, rng: &mut Rng) -> Option<&str> {
        if self.step.len() == 0 {
            return None;
        }

        Some(&self.step[rng.next_u32() as usize % self.step.len()])
    }

    pub fn hit(&self, rng: &mut Rng) -> Option<&str> {
        if self.hit.len() == 0 {
            return None;
        }

        Some(&self.hit[rng.next_u32() as usize % self.hit.len()])
    }

    pub fn destroy(&self, rng: &mut Rng) -> Option<&str> {
        if self.destroy.len() == 0 {
            return None;
        }

        Some(&self.destroy[rng.next_u32() as usize % self.destroy.len()])
    }

    pub fn place(&self, rng: &mut Rng) -> Option<&str> {
        if self.place.len() == 0 {
            return None;
        }

        Some(&self.place[rng.next_u32() as usize % self.place.len()])
    }
}

#[derive(Deserialize)]
struct Color {
    red: f32,
    green: f32,
    blue: f32,
    alpha: f32,
}

#[derive(Deserialize)]
#[serde(default)]
struct BlockMaterial {
    base_color: Color,
}

impl Default for BlockMaterial {
    fn default() -> Self {
        Self {
            base_color: Color {
                red: 1.0,
                green: 1.0,
                blue: 1.0,
                alpha: 1.0,
            },
        }
    }
}
