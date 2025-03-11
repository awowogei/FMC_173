use fmc::prelude::*;

mod duck;
mod pathfinding;
mod zombie;

pub struct MobsPlugin;
impl Plugin for MobsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(duck::DuckPlugin)
            .add_plugins(zombie::ZombiePlugin);
    }
}
