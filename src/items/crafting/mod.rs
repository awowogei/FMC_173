use fmc::{
    items::{ItemId, ItemStack, Items},
    prelude::*,
};
use serde::{Deserialize, Serialize};

use std::collections::HashMap;

mod shaped;

pub struct CraftingPlugin;
impl Plugin for CraftingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_recipes);
    }
}

fn load_recipes(mut commands: Commands, items: Res<Items>) {
    let mut recipes = HashMap::new();

    let directory = std::fs::read_dir("assets/client/items/recipes").expect(
        "Couldn't read recipe directory make sure it is present at: \
                assets/client/items/recipes",
    );

    for dir_entry in directory {
        let file_path = match dir_entry {
            Ok(d) => d.path(),
            Err(e) => panic!("Failed to read the filename of a recipe\nError: {}", e),
        };

        let file = match std::fs::File::open(&file_path) {
            Ok(f) => f,
            Err(e) => panic!(
                "Failed to open recipe at path: {}\nError: {}",
                &file_path.display(),
                e
            ),
        };

        let item_recipes: Vec<RecipeJson> = match serde_json::from_reader(file) {
            Ok(i) => i,
            Err(e) => panic!(
                "Failed to read item recipe in file: {}\nError:{}",
                file_path.display(),
                e
            ),
        };

        for recipe_json in item_recipes.into_iter() {
            match recipe_json.pattern_type.as_str() {
                "shaped" => {
                    let (pattern, required_amount): (Vec<Vec<Option<ItemId>>>, Vec<Vec<u32>>) =
                        match &recipe_json.pattern {
                            PatternJson::Grid(pattern) => pattern
                                .iter()
                                .map(|row| {
                                    row.iter()
                                        .map(|(name, amount)| match name.as_str() {
                                            // Empty part of pattern
                                            "" => (None, 0),
                                            // Item part of pattern
                                            _ => match items.get_id(name) {
                                                Some(id) => (Some(id), *amount),
                                                None => panic!(
                                                    "Error parsing item recipe pattern at: {}\n\
                                                        Item name '{}' is not recognized",
                                                    file_path.display(),
                                                    name
                                                ),
                                            },
                                        })
                                        .unzip()
                                })
                                .unzip(),
                            _ => panic!(
                                r#"Error parsing item recipe pattern at: {}
'pattern_type' is 'shaped', but the pattern is not in the form of a grid. Should be like:
[
       [["", 0], ["item", 1]],
       [["item", 1], ["", 0]]
]"#,
                                file_path.display()
                            ),
                        };

                    let output_config = match items.get_config_by_name(&recipe_json.output_item) {
                        Some(id) => id,
                        None => panic!(
                            "Error parsing item recipe pattern at: {}\n Item name '{}'\
                            is not recognized",
                            file_path.display(),
                            &recipe_json.output_item
                        ),
                    };

                    let recipe = shaped::Recipe {
                        required_amount,
                        output: ItemStack::new(output_config, recipe_json.output_amount),
                    };

                    recipes
                        .entry(recipe_json.collection_name)
                        .or_insert(RecipeCollection::default())
                        .insert(
                            Pattern::Shaped(shaped::Pattern { inner: pattern }),
                            Recipe::Shaped(recipe),
                        );
                }
                _ => (),
            }
        }
    }

    commands.insert_resource(Recipes {
        collections: recipes,
    })
}

/// A square crafting grid.
///
/// Used in combination with a [Recipe] to craft items
#[derive(Component, Deref, DerefMut, Serialize, Deserialize)]
pub struct CraftingGrid(Vec<ItemStack>);

impl CraftingGrid {
    /// Create a new square crafting grid with width/lenght == size
    pub fn with_size(size: usize) -> Self {
        let mut grid = Vec::with_capacity(size);
        grid.resize_with(size, ItemStack::default);

        Self(grid)
    }
}

#[derive(Serialize, Deserialize)]
struct RecipeJson {
    collection_name: String,
    pattern_type: String,
    pattern: PatternJson,
    output_item: String,
    output_amount: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum PatternJson {
    // Use an empty string to signify an open slot in the pattern
    Grid(Vec<Vec<(String, u32)>>),
    List(Vec<(String, u32)>),
}

#[derive(Serialize, Deserialize)]
enum RequiredJson {
    Grid(Vec<Vec<u32>>),
    List(Vec<u32>),
}

// TODO: Unshaped and Unordered are the same right? merge if
// All recipe types share the same interface, but differ in the way they handle the input.
pub enum Recipe {
    /// Square crafting area where the item position matters.
    Shaped(shaped::Recipe),
    //// Square crafting area where the item position doesn't matter.
    //Unshaped(UnshapedRecipe),
    //// List of crafting items where the order matters
    //Ordered(OrderedRecipe)
    //// List of crafting items where the order doesn't matter
    //Unordered(UnorderedRecipe)
}

impl Recipe {
    /// Craft items by consuming the input. Will produce ('amount' * the recipe amount) items
    /// (or as many as possible if amount is more than is possible).
    /// DOES NOT TEST THAT THE INPUT MATCHES
    pub fn craft(&self, input: &mut CraftingGrid, amount: u32) -> Option<ItemStack> {
        return match self {
            Recipe::Shaped(r) => r.craft(input, amount),
        };
    }

    /// Get how many of the output item can be crafted given the input.
    fn get_craftable_amount(&self, input: &CraftingGrid) -> u32 {
        return match self {
            Recipe::Shaped(r) => r.get_craftable_amount(input),
        };
    }

    pub fn output(&self) -> &ItemStack {
        match self {
            Recipe::Shaped(s) => s.output(),
        }
    }
}

#[derive(Hash, PartialEq, Eq)]
enum Pattern {
    Shaped(shaped::Pattern),
}

// TODO: I dislike the return types, 'craft' should have proper type, and 'check_output' should not
// hijack ItemStack. I can't come up with any good names at the moment.
//
/// A subset of recipes. e.g. the recipes that are used by a crafting table, or the recipes
/// that are used by a furnace.
#[derive(Default)]
pub struct RecipeCollection {
    shaped: bool,
    recipes: HashMap<Pattern, Recipe>,
}

impl RecipeCollection {
    fn insert(&mut self, pattern: Pattern, recipe: Recipe) {
        match pattern {
            Pattern::Shaped(_) => {
                self.shaped = true;
                self.recipes.insert(pattern, recipe);
            }
        }
    }

    pub fn craft(&self, input: &mut CraftingGrid, amount: u32) -> Option<ItemStack> {
        if self.shaped {
            let pattern = Pattern::Shaped(shaped::Pattern::from(input.as_slice()));
            let Some(recipe) = self.recipes.get(&pattern) else {
                return None;
            };
            return recipe.craft(input, amount);
        } else {
            todo!()
        }
    }

    /// Check what item can be crafted. The returned item stack uses its 'size' field to store the
    /// max amount of items that can be crafted, and its capacity to store how many items are
    /// crafted at once.
    pub fn get_output(&self, input: &CraftingGrid) -> Option<ItemStack> {
        if self.shaped {
            let pattern = Pattern::Shaped(shaped::Pattern::from(input.as_slice()));
            let Some(recipe) = self.recipes.get(&pattern) else {
                return None;
            };

            let max_craft = recipe.get_craftable_amount(input);
            if max_craft == 0 {
                return None;
            }

            let mut item_stack = recipe.output().clone();
            item_stack
                .set_size(max_craft)
                .set_capacity(recipe.output().size());

            return Some(item_stack);
        } else {
            todo!()
        }
    }

    pub fn get_recipe(&self, input: &CraftingGrid) -> Option<&Recipe> {
        if self.shaped {
            let pattern = Pattern::Shaped(shaped::Pattern::from(input.as_slice()));
            return self.recipes.get(&pattern);
        }
        return None;
    }
}

/// Holds all crafting recipes in the game.
///
/// The recipes are sorted into collections based on where they are used. For example "smelting"
/// for the furnace, or "crafting" for the crafting table.
#[derive(Resource)]
pub struct Recipes {
    collections: HashMap<String, RecipeCollection>,
}

impl Recipes {
    pub fn get(&self, collection_name: &str) -> &RecipeCollection {
        return match self.collections.get(collection_name) {
            Some(c) => c,
            None => panic!(
                "No recipes found for the collection name: {}",
                collection_name
            ),
        };
    }
}
