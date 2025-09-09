use fmc::{
    blocks::{BlockPosition, Blocks, Friction},
    models::{Model, ModelMap, ModelSystems, Models},
    networking::Server,
    physics::{Collider, shapes::Aabb},
    players::Player,
    prelude::*,
    protocol::messages,
    world::{
        ChangedBlockEvent, ChunkLoadEvent, ChunkOrigin, ChunkSubscriptions, WorldMap,
        chunk::{Chunk, ChunkPosition},
    },
};

use serde::Serialize;

pub(super) struct MovementPlugin;
impl Plugin for MovementPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, send_setup)
            .add_systems(Last, send_block_models.after(ModelSystems));
    }
}

#[derive(Serialize)]
pub enum MovementPluginPacket<'a> {
    Setup {
        blocks: Vec<CollisionConfig>,
        models: Vec<CollisionConfig>,
    },
    /// Changes the player's velocity
    Velocity(Vec3),
    /// Notifies the plugin of which models it should collide with.
    Models(&'a Vec<u32>),
    /// Changes the game mode
    GameMode(u32),
}

#[derive(Serialize)]
pub struct CollisionConfig {
    collider: Vec3Collider,
    friction: Vec3Friction,
    climbable: bool,
}

#[derive(Serialize)]
enum Vec3Collider {
    Single(Vec3Aabb),
    Multi(Vec<Vec3Aabb>),
}

impl From<&Collider> for Vec3Collider {
    fn from(collider: &Collider) -> Self {
        match collider {
            Collider::Single(aabb) => Vec3Collider::Single(aabb.into()),
            Collider::Multi(aabbs) => {
                Vec3Collider::Multi(aabbs.iter().map(|aabb| aabb.into()).collect())
            }
        }
    }
}

#[derive(Serialize)]
struct Vec3Aabb {
    center: Vec3,
    half_extents: Vec3,
}

impl From<&Aabb> for Vec3Aabb {
    fn from(aabb: &Aabb) -> Self {
        Vec3Aabb {
            center: aabb.center.as_vec3(),
            half_extents: aabb.half_extents.as_vec3(),
        }
    }
}

#[derive(Serialize)]
enum Vec3Friction {
    Surface {
        front: f32,
        back: f32,
        right: f32,
        left: f32,
        top: f32,
        bottom: f32,
    },
    Drag(Vec3),
}

impl From<&Friction> for Vec3Friction {
    fn from(friction: &Friction) -> Self {
        match friction {
            Friction::Surface {
                front,
                back,
                right,
                left,
                top,
                bottom,
            } => Vec3Friction::Surface {
                front: *front as f32,
                back: *back as f32,
                right: *right as f32,
                left: *left as f32,
                top: *top as f32,
                bottom: *bottom as f32,
            },
            Friction::Drag(drag) => Vec3Friction::Drag(drag.as_vec3()),
        }
    }
}

fn send_setup(net: Res<Server>, models: Res<Models>, new_players: Query<Entity, Added<Player>>) {
    for player_entity in new_players.iter() {
        // TODO: These can be pre-computed
        let block_collision_configs = Blocks::get()
            .configs()
            .iter()
            .map(|config| CollisionConfig {
                collider: Vec3Collider::from(&config.collider),
                friction: Vec3Friction::from(&config.friction),
                climbable: &config.name == "ladder",
            })
            .collect();

        let model_collision_configs = models
            .configs()
            .iter()
            .map(|config| CollisionConfig {
                collider: Vec3Collider::from(&config.collider),
                friction: Vec3Friction::Surface {
                    top: 0.99,
                    bottom: 0.0,
                    left: 0.0,
                    right: 0.0,
                    front: 0.0,
                    back: 0.0,
                },
                climbable: false,
            })
            .collect();

        net.send_one(
            player_entity,
            messages::PluginData {
                plugin: "movement".to_owned(),
                data: bincode::serialize(&MovementPluginPacket::Setup {
                    blocks: block_collision_configs,
                    models: model_collision_configs,
                })
                .unwrap(),
            },
        );
    }
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

                    if block_config.is_solid() {
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
