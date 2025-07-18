mod assets;
pub mod chat;
pub mod items;
pub mod mobs;
pub mod players;
pub mod settings;
pub mod skybox;
pub mod world;

pub use fmc;

pub mod prelude {
    #[doc(no_inline)]
    pub use fmc::prelude::*;
}

use fmc::bevy::app::{PluginGroup, PluginGroupBuilder};
pub struct DefaultPlugins;
impl PluginGroup for DefaultPlugins {
    fn build(self) -> fmc::bevy::app::PluginGroupBuilder {
        let group = PluginGroupBuilder::start::<Self>();
        group
            // This must run first so all the expected assets are present
            .add(assets::ExtractBundledAssetsPlugin)
            .add_group(fmc::DefaultPlugins)
            .add(settings::SettingsPlugin)
            .add(items::ItemPlugin)
            .add(players::PlayerPlugin)
            .add(world::WorldPlugin)
            .add(skybox::SkyPlugin)
            .add(mobs::MobsPlugin)
            .add(chat::ChatPlugin)
    }
}
