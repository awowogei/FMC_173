use fmc::{
    audio::{AudioPlayer, Sound},
    blocks::BlockId,
    items::{Item, ItemStack},
    math::DVec3,
    players::{Player, Target, Targets},
    prelude::*,
    random::Rng,
    world::BlockUpdate,
};

use super::{DroppedItem, ItemEvents, Items};
use crate::world::blocks::Blocks;

pub struct HoePlugin;
impl Plugin for HoePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, register_hoes)
            .add_systems(Update, use_hoe.after(super::ItemSystems));
    }
}

fn register_hoes(mut commands: Commands, blocks: Res<Blocks>, items: Res<Items>) {
    for hoe in ["wooden_hoe", "stone_hoe", "iron_hoe"] {
        let item_id = items.get_id(hoe).unwrap();
        commands.spawn((
            ItemEvents::new(item_id),
            HoeConfig {
                dirt: blocks.get_id("dirt"),
                grass: blocks.get_id("grass"),
            },
        ));
    }
}

#[derive(Component)]
struct HoeConfig {
    pub dirt: BlockId,
    pub grass: BlockId,
}

fn use_hoe(
    mut commands: Commands,
    audio: AudioPlayer,
    blocks: Res<Blocks>,
    items: Res<Items>,
    player_query: Query<&Targets, With<Player>>,
    mut hoe_query: Query<(&mut ItemEvents, &HoeConfig), Changed<ItemEvents>>,
    mut block_update_writer: MessageWriter<BlockUpdate>,
    mut rng: Local<Rng>,
) {
    let Ok((mut events, config)) = hoe_query.single_mut() else {
        return;
    };

    for player_entity in events.read() {
        let targets = player_query.get(player_entity).unwrap();

        let Some(Target::Block {
            block_position,
            block_id,
            ..
        }) = targets
            .get_first_block(|block_id| *block_id == config.dirt || *block_id == config.grass)
        else {
            continue;
        };

        if *block_id == config.grass {
            let item_config = items.get_config_by_name("wheat_seeds").unwrap();
            let item_stack = ItemStack::new(item_config.id, 1);
            commands.spawn((
                DroppedItem::new(item_stack),
                Transform::from_translation(block_position.as_dvec3() + DVec3::new(0.5, 1.1, 0.5)),
            ));
        }

        let soil_id = blocks.get_id("soil");
        let block_config = blocks.get_config(&soil_id);

        if let Some(place_sound) = block_config.sound.place(&mut rng) {
            let position = block_position.as_dvec3() + DVec3::splat(0.5);
            audio.play_at(position, Sound::new(place_sound));
        }

        block_update_writer.write(BlockUpdate::Replace {
            position: *block_position,
            block_id: soil_id,
            block_state: None,
            block_data: None,
        });
    }
}
