use fmc::prelude::*;

mod chest;
mod crafting_table;
mod furnace;
mod torch;
mod water;
mod wheat;

pub(super) struct BlocksPlugin;
impl Plugin for BlocksPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(crafting_table::CraftingTablePlugin)
            .add_plugins(chest::ChestPlugin)
            .add_plugins(furnace::FurnacePlugin)
            .add_plugins(torch::TorchPlugin)
            .add_plugins(water::WaterPlugin)
            .add_plugins(wheat::WheatPlugin);
    }
}
