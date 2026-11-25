use fmc::{
    items::ItemId,
    players::{Camera, Player, Targets},
    prelude::*,
};

use crate::{
    mobs::{Mob, MobId, Mobs},
    world::blocks::Blocks,
};

use super::{ItemAction, ItemEvents};

pub struct CratePlugin;
impl Plugin for CratePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(MobCrates::default())
            .add_systems(PostStartup, register_crates)
            .add_systems(Update, use_crate.after(super::ItemSystems));
    }
}

#[derive(Resource, Default)]
pub struct MobCrates {
    crates: Vec<(ItemId, MobId)>,
}

impl MobCrates {
    pub fn add_crate(&mut self, item_id: ItemId, mob_id: MobId) {
        self.crates.push((item_id, mob_id));
    }
}

#[derive(Component)]
struct MobCrate {
    mob_id: MobId,
}

fn register_crates(mut commands: Commands, mob_crates: Res<MobCrates>) {
    for (item_id, mob_id) in mob_crates.crates.iter().cloned() {
        commands.spawn((ItemEvents::new(item_id), MobCrate { mob_id }));
    }
}

fn use_crate(
    mut commands: Commands,
    mobs: Res<Mobs>,
    blocks: Res<Blocks>,
    player_query: Query<(&GlobalTransform, &Camera, &Targets), With<Player>>,
    mut crate_query: Query<(&mut ItemEvents, &MobCrate), Changed<ItemEvents>>,
) {
    for (mut events, mob_crate) in crate_query.iter_mut() {
        let mob_config = mobs.get_config(mob_crate.mob_id);

        for event in events.read() {
            if event.action != ItemAction::Use {
                continue;
            }

            let (transform, camera, targets) = player_query.get(event.player_entity).unwrap();

            let Some(target) =
                targets.get_first_block(|block_id| blocks.get_config(block_id).is_solid())
            else {
                continue;
            };

            let spawn_position = transform.translation()
                + camera.translation
                + camera.forward() * target.distance();

            let mut entity_commands = commands.spawn((
                Mob {
                    id: mob_crate.mob_id,
                },
                Transform::from_translation(spawn_position),
            ));

            (mob_config.spawn_function)(&mut entity_commands);
        }
    }
}
