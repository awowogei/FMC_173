use fmc::{
    blocks::{BlockPosition, Blocks},
    models::{Model, ModelMap, ModelSystems},
    networking::Server,
    players::Player,
    prelude::*,
    protocol::messages,
    world::{
        chunk::{Chunk, ChunkPosition},
        ChangedBlockEvent, ChunkLoadEvent, ChunkOrigin, ChunkSubscriptions, WorldMap,
    },
};
use serde::Serialize;

pub(super) struct MovementPlugin;
impl Plugin for MovementPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Last, send_block_models.after(ModelSystems));
    }
}

#[derive(Serialize)]
pub enum MovementPluginPacket<'a> {
    /// Changes the player's velocity
    Velocity(Vec3),
    /// Notifies the plugin of which models it should collide with.
    Models(&'a Vec<u32>),
}

fn send_block_models(
    net: Res<Server>,
    world_map: Res<WorldMap>,
    model_map: Res<ModelMap>,
    chunk_subscriptions: Res<ChunkSubscriptions>,
    block_model_query: Query<&BlockPosition, With<Model>>,
    players: Query<(Entity, Ref<ChunkOrigin>), With<Player>>,
    mut changed_blocks: EventReader<ChangedBlockEvent>,
    mut loaded_chunks: EventReader<ChunkLoadEvent>,
    mut nearby_models: Local<Vec<u32>>,
) {
    let gather_models = |chunk_position: ChunkPosition, nearby_models: &mut Vec<u32>| {
        nearby_models.clear();

        for chunk_position in chunk_position.neighbourhood() {
            if let Some(models) = model_map.get_entities(&chunk_position) {
                for model_entity in models {
                    let Ok(block_position) = block_model_query.get(*model_entity) else {
                        continue;
                    };
                    let block_id = world_map.get_block(*block_position).unwrap();
                    let block_config = Blocks::get().get_config(&block_id);

                    if block_config.friction.is_some() {
                        nearby_models.push(model_entity.index());
                    }
                }
            }
        }
    };

    // When a player move over a chunk boundary
    for (player_entity, player_chunk_origin) in players.iter() {
        if !player_chunk_origin.is_changed() {
            continue;
        }

        gather_models(player_chunk_origin.chunk_position, &mut nearby_models);

        if nearby_models.is_empty() {
            continue;
        }

        net.send_one(
            player_entity,
            messages::PluginData {
                plugin: "movement".to_owned(),
                data: bincode::serialize(&MovementPluginPacket::Models(&nearby_models)).unwrap(),
            },
        )
    }

    for block_update in changed_blocks.read() {
        let from = Blocks::get().get_config(&block_update.from.0);
        let to = Blocks::get().get_config(&block_update.to.0);

        if !from.model.is_some() && to.model.is_some() {
            continue;
        }

        let model_chunk_position = ChunkPosition::from(block_update.position);
        let Some(subscribers) = chunk_subscriptions.get_subscribers(&model_chunk_position) else {
            continue;
        };

        for player_entity in subscribers.iter() {
            let (_, player_origin) = players.get(*player_entity).unwrap();
            if (player_origin.chunk_position - model_chunk_position)
                .abs()
                .cmple(IVec3::splat(Chunk::SIZE as i32))
                .all()
            {
                gather_models(player_origin.chunk_position, &mut nearby_models);
                net.send_one(
                    *player_entity,
                    messages::PluginData {
                        plugin: "movement".to_owned(),
                        data: bincode::serialize(&MovementPluginPacket::Models(&nearby_models))
                            .unwrap(),
                    },
                )
            }
        }
    }

    for new_chunk in loaded_chunks.read() {
        let Some(subscribers) = chunk_subscriptions.get_subscribers(&new_chunk.position) else {
            continue;
        };

        for player_entity in subscribers.iter() {
            let (_, player_origin) = players.get(*player_entity).unwrap();
            if (player_origin.chunk_position - new_chunk.position)
                .abs()
                .cmple(IVec3::splat(Chunk::SIZE as i32))
                .all()
            {
                gather_models(player_origin.chunk_position, &mut nearby_models);
                net.send_one(
                    *player_entity,
                    messages::PluginData {
                        plugin: "movement".to_owned(),
                        data: bincode::serialize(&MovementPluginPacket::Models(&nearby_models))
                            .unwrap(),
                    },
                )
            }
        }
    }
}
