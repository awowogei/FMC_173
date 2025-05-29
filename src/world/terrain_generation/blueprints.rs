use fmc::prelude::*;
use rand::{distributions::Distribution, Rng};

use serde::Deserialize;
use std::collections::{HashMap, HashSet};

use fmc::{
    blocks::{BlockId, BlockPosition, Blocks, BLOCK_CONFIG_PATH},
    world::{
        chunk::{Chunk, ChunkPosition},
        Surface, TerrainFeature,
    },
};

pub const BLUEPRINT_PATH: &str = "./assets/server/blueprints/";

/// Blueprints contain instructions for placing terrain features.
/// Even though blueprints are mainly intended to compose features, some features are so common that
/// they get their own blueprint variant. All other custom features should be coded as a
/// [Blueprint::Generator]
#[derive(Clone)]
pub enum Blueprint {
    // A collection of blueprints that will be generated together.
    Collection(Vec<Blueprint>),
    Distribution {
        // The blueprint that should be distributed.
        blueprint: Box<Blueprint>,
        // TODO: This is just a uniform distribution now. Triangle distributions would be nice, but the
        // rand crate implements distributions as a trait which makes them difficult to store since
        // each distribution type has its own struct. Rand is also slow, will probably have to do
        // some homebrew.
        //
        // Number of attempts at placing that should be done for each chunk
        count: u32,
        // If specified it will only distribute between the two height values. If None, it will
        // snap to the surface. [low_y, high_y]
        vertical_range: Option<[i32; 2]>,
    },
    // A function that generates a feature
    Generator(fn(position: BlockPosition, chunk: &mut Chunk)),
    // Places a single block
    Decoration {
        decoration_block: BlockId,
        placed_on: HashSet<BlockId>,
        can_replace: HashSet<BlockId>,
    },
    Tree(Tree),
    OreVein {
        /// The block that is placed
        ore_block: BlockId,
        /// The number of ore blocks that are placed.
        count: u32,
        /// Which blocks the ore can be placed into.
        can_replace: HashSet<BlockId>,
    },
}

impl Blueprint {
    fn new(
        json_blueprint: &AmbiguousJsonBlueprint,
        named_blueprints: &HashMap<String, AmbiguousJsonBlueprint>,
        blocks: &Blocks,
    ) -> Self {
        match json_blueprint {
            AmbiguousJsonBlueprint::Named(name) => Blueprint::new(
                named_blueprints.get(name).unwrap(),
                named_blueprints,
                blocks,
            ),
            AmbiguousJsonBlueprint::Inline(json_blueprint) => match json_blueprint {
                JsonBlueprint::Collection { blueprints, .. } => {
                    let mut collection = Vec::with_capacity(blueprints.len());
                    for sub_blueprint in blueprints {
                        let sub_blueprint = Blueprint::new(sub_blueprint, named_blueprints, blocks);
                        collection.push(sub_blueprint);
                    }
                    Blueprint::Collection(collection)
                }
                JsonBlueprint::Distribution {
                    blueprint,
                    count,
                    vertical_range,
                } => {
                    let sub_blueprint = Blueprint::new(blueprint, named_blueprints, blocks);
                    Blueprint::Distribution {
                        blueprint: Box::new(sub_blueprint),
                        count: *count,
                        vertical_range: vertical_range.clone(),
                    }
                }
                JsonBlueprint::Decoration {
                    decoration_block,
                    placed_on,
                    can_replace,
                } => Blueprint::Decoration {
                    decoration_block: blocks.get_id(&decoration_block),
                    placed_on: placed_on
                        .iter()
                        .map(|block_name| blocks.get_id(block_name))
                        .collect::<HashSet<BlockId>>(),
                    can_replace: can_replace
                        .iter()
                        .map(|block_name| blocks.get_id(block_name))
                        .collect::<HashSet<BlockId>>(),
                },
                JsonBlueprint::Tree {
                    trunk_block,
                    leaf_block,
                    trunk_height,
                    foliage_style,
                    branches,
                    random_height,
                    trunk_width,
                    soil_blocks,
                    can_replace,
                } => Blueprint::Tree(Tree {
                    trunk_block: blocks.get_id(&trunk_block),
                    foliage_style: match foliage_style {
                        FoliageStyleJson::Normal => FoliageStyle::Normal {
                            clipper: rand::distributions::Bernoulli::new(0.5).unwrap(),
                            leaf_block: blocks.get_id(&leaf_block),
                        },
                        FoliageStyleJson::Blob { radius } => FoliageStyle::Blob {
                            radius: *radius,
                            leaf_block: blocks.get_id(&leaf_block),
                        },
                    },
                    branches: match branches {
                        Some(b) => rand::distributions::Uniform::new(b[0], b[1]),
                        None => rand::distributions::Uniform::new(0, 1),
                    },
                    trunk_height: *trunk_height as i32,
                    random_height: rand::distributions::Uniform::new_inclusive(
                        0,
                        random_height.unwrap_or(0) as i32,
                    ),
                    trunk_width: *trunk_width,
                    soil_blocks: soil_blocks
                        .iter()
                        .map(|block_name| blocks.get_id(block_name))
                        .collect::<HashSet<BlockId>>(),
                    can_replace: can_replace
                        .iter()
                        .map(|block_name| blocks.get_id(block_name))
                        .collect::<HashSet<BlockId>>(),
                }),
                JsonBlueprint::OreVein {
                    ore_block,
                    count,
                    can_replace,
                } => Blueprint::OreVein {
                    ore_block: blocks.get_id(&ore_block),
                    count: *count,
                    can_replace: can_replace
                        .iter()
                        .map(|block_name| blocks.get_id(block_name))
                        .collect::<HashSet<BlockId>>(),
                },
            },
        }
    }

    // TODO: The surface parameter is too constricting, a blueprint might want to know all open
    // faces be it floor, roof or wall, below or above ground. Idk how to do it.
    pub fn construct(
        &self,
        origin: BlockPosition,
        chunk: &mut Chunk,
        surface: &Surface,
        rng: &mut rand::rngs::StdRng,
    ) {
        match self {
            Blueprint::Collection(blueprints) => {
                for blueprint in blueprints {
                    blueprint.construct(origin, chunk, surface, rng);
                }
            }
            Blueprint::Distribution {
                blueprint,
                count,
                vertical_range,
            } => {
                if let Some(vertical_range) = vertical_range {
                    if vertical_range[0] > origin.y || vertical_range[1] < origin.y {
                        return;
                    }
                };

                let distribution = rand::distributions::Uniform::new(0, Chunk::SIZE.pow(3));
                for _ in 0..*count {
                    let position = origin + BlockPosition::from(rng.sample(distribution));
                    blueprint.construct(position, chunk, surface, rng);
                }
            }
            Blueprint::Decoration {
                decoration_block,
                placed_on,
                can_replace,
            } => {
                let mut terrain_feature = TerrainFeature::default();
                terrain_feature.can_replace.extend(can_replace);

                let chunk_position = ChunkPosition::from(origin);
                let index = origin.as_chunk_index();
                let index = index >> 4;
                let (surface_y, surface_block) = match &surface[index] {
                    Some(s) => s,
                    None => return,
                };

                if !placed_on.contains(surface_block) {
                    return;
                }

                let mut position = origin;
                position.y = chunk_position.y + *surface_y as i32 + 1;

                terrain_feature.insert_block(position, *decoration_block);

                terrain_feature.apply(chunk_position, chunk);
            }
            Blueprint::Generator(generator_function) => {
                generator_function(origin, chunk);
            }
            Blueprint::Tree(tree) => {
                let mut terrain_feature = TerrainFeature::default();

                // The distribution goes over a 3d space, so we convert it to 2d and set the y to
                // whatever the surface height is at that position.
                let chunk_position = ChunkPosition::from(origin);
                let index = origin.as_chunk_index();
                let index = index >> 4;
                let (surface_y, surface_block) = match &surface[index] {
                    Some(s) => s,
                    None => return,
                };

                let mut trunk_position = origin;
                trunk_position.y = chunk_position.y + *surface_y as i32;

                tree.construct(trunk_position, *surface_block, &mut terrain_feature, rng);

                terrain_feature.apply(chunk_position, chunk);
            }
            Blueprint::OreVein {
                ore_block,
                count,
                can_replace,
            } => {
                let mut terrain_feature = TerrainFeature::default();

                // TODO: Implement as const when making rand lib
                let directions = rand::distributions::Slice::<IVec3>::new(&[
                    IVec3::X,
                    IVec3::NEG_X,
                    IVec3::Y,
                    IVec3::NEG_Y,
                    IVec3::Z,
                    IVec3::NEG_Z,
                ])
                .unwrap();

                let mut position = origin;
                for direction in directions.sample_iter(rng).take(*count as usize) {
                    position += *direction;
                    terrain_feature.insert_block(position, *ore_block)
                }

                terrain_feature.can_replace.extend(can_replace);

                terrain_feature.apply(ChunkPosition::from(origin), chunk);
            }
        }
    }
}

// This allows json blueprints to be nested in an ergonomic way in exchange for less ergonomic
// code.
//
// named:
// {
//     type: some_blueprint_type,
//     field_1: some_value,
//     nested_blueprint: "blueprint_1"
// }
//
// inline:
// {
//     type: some_blueprint_type
//     field_1: some_value,
//     nested_blueprint: {
//         type: some_blueprint_type,
//         field_1: some_value,
//         ...
//     }
// }
#[derive(Deserialize)]
#[serde(untagged)]
enum AmbiguousJsonBlueprint {
    Named(String),
    Inline(JsonBlueprint),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum JsonBlueprint {
    Collection {
        blueprints: Vec<AmbiguousJsonBlueprint>,
    },
    Distribution {
        blueprint: Box<AmbiguousJsonBlueprint>,
        count: u32,
        vertical_range: Option<[i32; 2]>,
    },
    Decoration {
        decoration_block: String,
        placed_on: Vec<String>,
        can_replace: Vec<String>,
    },
    Tree {
        trunk_block: String,
        leaf_block: String,
        trunk_height: u32,
        foliage_style: FoliageStyleJson,
        branches: Option<[u32; 2]>,
        random_height: Option<u32>,
        trunk_width: u32,
        soil_blocks: Vec<String>,
        can_replace: Vec<String>,
    },
    OreVein {
        ore_block: String,
        count: u32,
        can_replace: Vec<String>,
    },
}

pub fn load_blueprints(blocks: &Blocks) -> HashMap<String, Blueprint> {
    let mut named_json_blueprints = HashMap::new();

    let directory = std::fs::read_dir(BLUEPRINT_PATH).expect(&format!(
        "Could not read files from blueprints directory, make sure it is present as '{}'",
        BLUEPRINT_PATH
    ));

    for entry in directory {
        let file_path = entry
            .expect("Failed to read the filenames of the block configs")
            .path();

        let file = std::fs::File::open(&file_path).expect(&format!(
            "Failed to open blueprint file at '{}'",
            file_path.display()
        ));
        let blueprint = serde_json::from_reader(file).expect(&format!(
            "Failed to read blueprint at '{}'",
            file_path.display()
        ));
        let name = file_path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();

        named_json_blueprints.insert(name, blueprint);
    }

    fn validate_blueprint(
        parent_name: &str,
        child_name: &str,
        named_blueprints: &HashMap<String, AmbiguousJsonBlueprint>,
    ) {
        if !named_blueprints.contains_key(child_name) {
            panic!(
                "Failed while validating the terrain feature blueprints. The blueprint '{}', \
                depends on another blueprint '{}', but it could not be found. This is most \
                likely the result of a missing file at '{}', make sure it is present.",
                parent_name,
                child_name,
                BLUEPRINT_PATH.to_owned() + child_name + ".json"
            );
        }
    }

    fn validate_block(blueprint_name: &str, block_name: &str, blocks: &Blocks) {
        if !blocks.contains_block(block_name) {
            panic!(
                "Failed while validating the terrain feature blueprints. The blueprint '{}' \
                references a block with the name '{}', but no block by that name exists. \
                Make sure a block by the same name is present at '{}'",
                blueprint_name, block_name, BLOCK_CONFIG_PATH
            );
        }
    }

    for (blueprint_name, json_blueprint) in named_json_blueprints.iter() {
        match json_blueprint {
            AmbiguousJsonBlueprint::Named(child_name) => {
                validate_blueprint(blueprint_name, child_name, &named_json_blueprints)
            }
            AmbiguousJsonBlueprint::Inline(json_blueprint) => match json_blueprint {
                JsonBlueprint::Collection { blueprints } => {
                    for child_blueprint in blueprints {
                        if let AmbiguousJsonBlueprint::Named(child_name) = child_blueprint {
                            validate_blueprint(blueprint_name, child_name, &named_json_blueprints)
                        }
                    }
                }
                JsonBlueprint::Distribution { blueprint, .. } => {
                    if let AmbiguousJsonBlueprint::Named(child_name) = blueprint.as_ref() {
                        validate_blueprint(blueprint_name, child_name, &named_json_blueprints)
                    }
                }
                JsonBlueprint::Decoration {
                    decoration_block,
                    placed_on,
                    can_replace,
                } => {
                    validate_block(blueprint_name, &decoration_block, blocks);
                    for block_name in placed_on.iter() {
                        validate_block(blueprint_name, block_name, blocks)
                    }
                    for block_name in can_replace.iter() {
                        validate_block(blueprint_name, block_name, blocks)
                    }
                }
                JsonBlueprint::Tree {
                    trunk_block,
                    leaf_block,
                    soil_blocks,
                    can_replace,
                    ..
                } => {
                    validate_block(blueprint_name, &trunk_block, blocks);
                    validate_block(blueprint_name, &leaf_block, blocks);
                    for block_name in soil_blocks.iter() {
                        validate_block(blueprint_name, block_name, blocks)
                    }
                    for block_name in can_replace.iter() {
                        validate_block(blueprint_name, block_name, blocks)
                    }
                }
                JsonBlueprint::OreVein {
                    ore_block,
                    can_replace,
                    ..
                } => {
                    validate_block(blueprint_name, &ore_block, blocks);
                    for block_name in can_replace.iter() {
                        validate_block(blueprint_name, block_name, blocks)
                    }
                }
            },
        }
    }

    let mut blueprints = HashMap::new();

    for (name, json_blueprint) in named_json_blueprints.iter() {
        let blueprint = Blueprint::new(&json_blueprint, &named_json_blueprints, blocks);
        blueprints.insert(name.to_owned(), blueprint);
    }

    return blueprints;
}

#[derive(Clone)]
struct Tree {
    // Block used as trunk
    trunk_block: BlockId,
    // Foliage style the leaves are placed in
    foliage_style: FoliageStyle,
    // How many branches the tree should have
    branches: rand::distributions::Uniform<u32>,
    // Minimum height of the tree
    trunk_height: i32,
    // A random integer between 0 and random_height is added to the trunk height.
    random_height: rand::distributions::Uniform<i32>,
    // How many blocks wide the trunk should be
    trunk_width: u32,
    // Which blocks the tree can grow from.
    soil_blocks: HashSet<BlockId>,
    // Which blocks the tree can replace when it grows.
    can_replace: HashSet<BlockId>,
}

impl Tree {
    fn branches(
        &self,
        trunk_position: BlockPosition,
        height: i32,
        terrain_feature: &mut TerrainFeature,
        rng: &mut rand::rngs::StdRng,
    ) {
        // The lowest point on the trunk a branch can start at
        let branch_base = (height as f32 * 0.6) as i32;
        let branch_sampler = rand::distributions::Uniform::new_inclusive(branch_base, height);

        let rotation_sampler =
            rand::distributions::Uniform::new_inclusive(0.0, std::f32::consts::PI * 2.0);

        let max_branch_length = height - branch_base;
        let length_sampler = rand::distributions::Uniform::new_inclusive(
            3.min(max_branch_length),
            max_branch_length,
        );

        for _ in 0..self.branches.sample(rng) {
            let branch_height = branch_sampler.sample(rng);
            let branch_rotation = rotation_sampler.sample(rng);
            let branch_length = length_sampler.sample(rng);
            let x = f32::cos(branch_rotation);
            let z = f32::sin(branch_rotation);
            let branch_increment = Vec3::new(x, 0.4, z);

            for i in 1..=branch_length {
                let branch_position = trunk_position
                    + BlockPosition::new(0, branch_height, 0)
                    + BlockPosition::from(branch_increment * i as f32);
                terrain_feature.insert_block(branch_position, self.trunk_block);
            }

            let branch_tip = trunk_position
                + BlockPosition::new(0, branch_height, 0)
                + BlockPosition::from(branch_increment * branch_length as f32);

            self.foliage_style.place(branch_tip, terrain_feature, rng);
        }
    }

    fn construct(
        &self,
        trunk_position: BlockPosition,
        surface_block: BlockId,
        mut terrain_feature: &mut TerrainFeature,
        rng: &mut rand::rngs::StdRng,
    ) {
        if !self.soil_blocks.contains(&surface_block) {
            return;
        }

        terrain_feature.can_replace.extend(&self.can_replace);

        // Construct the trunk
        let trunk_height = self.trunk_height + self.random_height.sample(rng);
        for height in 1..=trunk_height {
            terrain_feature.insert_block(
                trunk_position + BlockPosition::new(0, height, 0),
                self.trunk_block,
            );
        }

        // Trunk bounding box
        terrain_feature.add_bounding_box(
            trunk_position + IVec3::Y,
            trunk_position + IVec3::new(0, trunk_height, 0),
        );

        let trunk_end = trunk_position + BlockPosition::new(0, trunk_height, 0);
        self.foliage_style
            .place(trunk_end, &mut terrain_feature, rng);

        self.branches(trunk_position, trunk_height, terrain_feature, rng);
    }
}

#[derive(Clone)]
enum FoliageStyle {
    Normal {
        // Clips leaves off the top
        clipper: rand::distributions::Bernoulli,
        leaf_block: BlockId,
    },
    Blob {
        radius: i32,
        leaf_block: BlockId,
    },
}

impl FoliageStyle {
    fn place(
        &self,
        branch_tip: BlockPosition,
        terrain_feature: &mut TerrainFeature,
        rng: &mut rand::rngs::StdRng,
    ) {
        match self {
            Self::Normal {
                clipper,
                leaf_block,
            } => {
                // Insert two bottom leaf layers.
                for y in -1..=0 {
                    for x in -2..=2 {
                        for z in -2..=2 {
                            if (x == 2 || x == -2) && (z == 2 || z == -2) && clipper.sample(rng) {
                                // Remove 50% of edges for more variance
                                continue;
                            }
                            terrain_feature.insert_block(
                                branch_tip + BlockPosition::new(x, y, z),
                                *leaf_block,
                            );
                        }
                    }
                }

                // Insert top layer of leaves.
                for y in 1..=2 {
                    for x in -1..=1 {
                        for z in -1..=1 {
                            if (x == 1 || x == -1) && (z == 1 || z == -1) && clipper.sample(rng) {
                                continue;
                            }
                            terrain_feature.insert_block(
                                branch_tip + BlockPosition::new(x, y, z),
                                *leaf_block,
                            );
                        }
                    }
                }

                // Foliage bounding box
                terrain_feature.add_bounding_box(
                    branch_tip - IVec3::new(1, 2, 1),
                    branch_tip + IVec3::new(1, 2, 1),
                );
            }
            Self::Blob { radius, leaf_block } => {
                let radius = *radius - 1;
                for (y, height) in (-radius..=radius).enumerate() {
                    // Trig trickery to get the radius of the circular cross section at that height.
                    let inner_radius = (f32::sin(f32::acos(height as f32 / radius as f32))
                        * radius as f32)
                        .round()
                        .max(1.0) as i32;
                    for x in -inner_radius..=inner_radius {
                        for z in -inner_radius..=inner_radius {
                            let block_position =
                                branch_tip + BlockPosition::new(x, y as i32 - 1, z);
                            terrain_feature.insert_block(block_position, *leaf_block);
                        }
                    }
                }
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum FoliageStyleJson {
    Normal,
    Blob { radius: i32 },
}
