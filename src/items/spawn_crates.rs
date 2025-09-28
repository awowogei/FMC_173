use fmc::{
    blocks::Blocks,
    items::ItemId,
    players::{Camera, Player, Targets},
    prelude::*,
};

use crate::mobs::{Mob, MobId, Mobs};

use super::{ItemRegistry, ItemUses};

pub struct CratePlugin;
impl Plugin for CratePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(MobCrates::default())
            .add_systems(PostStartup, register_crates)
            .add_systems(Update, use_crate.after(super::ItemUseSystems));
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

fn register_crates(
    mut commands: Commands,
    mob_crates: Res<MobCrates>,
    mut item_registry: ResMut<ItemRegistry>,
) {
    for (item_id, mob_id) in mob_crates.crates.iter().cloned() {
        item_registry.insert(
            item_id,
            commands
                .spawn((ItemUses::default(), MobCrate { mob_id }))
                .id(),
        );
    }
}

fn use_crate(
    mut commands: Commands,
    mobs: Res<Mobs>,
    player_query: Query<(&GlobalTransform, &Camera, &Targets), With<Player>>,
    mut crate_uses: Query<(&mut ItemUses, &MobCrate), Changed<ItemUses>>,
) {
    let Ok((mut uses, mob_crate)) = crate_uses.single_mut() else {
        return;
    };

    let mob_config = mobs.get_config(mob_crate.mob_id);

    for player_entity in uses.read() {
        let (transform, camera, targets) = player_query.get(player_entity).unwrap();

        let blocks = Blocks::get();
        let Some(target) =
            targets.get_first_block(|block_id| blocks.get_config(block_id).is_solid())
        else {
            continue;
        };

        let spawn_position =
            transform.translation() + camera.translation + camera.forward() * target.distance();

        let mut entity_commands = commands.spawn((
            Mob {
                id: mob_crate.mob_id,
            },
            Transform::from_translation(spawn_position),
        ));

        (mob_config.spawn_function)(&mut entity_commands);
    }
}
