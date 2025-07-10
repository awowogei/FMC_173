use std::collections::{HashMap, HashSet};

use fmc::{
    bevy::math::DVec3,
    blocks::{BlockConfig, BlockFace, BlockId, BlockPosition, Blocks},
    items::{ItemStack, Items},
    models::{AnimationPlayer, Model, ModelConfig, ModelMap, ModelVisibility, Models},
    networking::{NetworkMessage, Server},
    physics::{shapes::Aabb, Collider},
    players::{Camera, Player, Target, Targets},
    prelude::*,
    protocol::messages,
    utils::Rng,
    world::{chunk::ChunkPosition, BlockUpdate, ChunkSubscriptions, WorldMap},
};

use crate::{
    items::{DroppedItem, ItemRegistry, ItemUseSystems, ItemUses},
    players::Inventory,
};

pub struct HandPlugin;
impl Plugin for HandPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(MiningEvents::default()).add_systems(
            Update,
            (
                handle_left_clicks,
                handle_right_clicks.in_set(ItemUseSystems),
                break_blocks.after(handle_left_clicks),
            ),
        );
    }
}

/// Component that tracks when a player right clicks the entity
#[derive(Component, Default)]
pub struct HandInteractions {
    player_entities: Vec<Entity>,
}

impl HandInteractions {
    pub fn read(&mut self) -> impl Iterator<Item = Entity> + '_ {
        self.player_entities.drain(..)
    }

    pub fn push(&mut self, player_entity: Entity) {
        self.player_entities.push(player_entity);
    }
}

/// Component that tracks when a player left clicks the entity
#[derive(Component, Default)]
pub struct HandHits {
    player_entities: HashSet<Entity>,
}

impl HandHits {
    pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ {
        self.player_entities.iter().cloned()
    }

    pub fn push(&mut self, player_entity: Entity) {
        self.player_entities.insert(player_entity);
    }
}

fn handle_left_clicks(
    mut clicks: EventReader<NetworkMessage<messages::LeftClick>>,
    models: Res<Models>,
    mut player_query: Query<
        (&Targets, &Camera, &GlobalTransform, &mut AnimationPlayer),
        With<Player>,
    >,
    mut hittable_entities: Query<(&mut HandHits, Option<&ModelVisibility>)>,
    mut mining_events: ResMut<MiningEvents>,
    mut click_tracker: Local<HashSet<Entity>>,
) {
    for (mut hand_hits, _) in hittable_entities.iter_mut() {
        hand_hits.player_entities.clear();
    }

    for click in clicks.read() {
        let (targets, camera, transform, mut animation_player) =
            player_query.get_mut(click.player_entity).unwrap();

        let camera_position = transform.translation() + camera.translation;

        let model = models.get_by_name("player");
        let animation = animation_player.play(model.animations["hit"]);

        let mut first_click = false;
        if click.message == messages::LeftClick::Release {
            click_tracker.remove(&click.player_entity);
        } else if click_tracker.insert(click.player_entity) {
            first_click = true;
            // If it is a fresh click we always restart the animation. More responsive and let's
            // you use it to signal others by clicking fast.
            animation.restart();
        }

        for target in targets.iter() {
            match target {
                Target::Block {
                    block_position,
                    block_id,
                    block_face,
                    distance,
                    entity,
                } => {
                    let block_config = Blocks::get().get_config(block_id);

                    if block_config.hardness.is_some() {
                        let hit_position = camera_position + camera.forward() * *distance;
                        mining_events.insert(
                            *block_position,
                            (
                                click.player_entity,
                                *block_id,
                                *block_face,
                                hit_position,
                                *entity,
                            ),
                        );

                        break;
                    }
                }
                Target::Entity { entity, .. } if first_click => {
                    if let Ok((mut hits, maybe_visibility)) = hittable_entities.get_mut(*entity) {
                        if matches!(maybe_visibility, Some(ModelVisibility::Hidden)) {
                            continue;
                        }
                        hits.push(click.player_entity);
                    }
                }
                _ => continue,
            }
        }
    }
}

#[derive(Resource, Deref, DerefMut, Default, Debug)]
struct MiningEvents(HashMap<BlockPosition, (Entity, BlockId, BlockFace, DVec3, Option<Entity>)>);

// Keeps the state of how far along a block is to breaking
#[derive(Debug)]
struct BreakingBlock {
    model_entity: Entity,
    progress: f32,
    prev_hit: std::time::Instant,
    particle_timer: Timer,
}

#[derive(Component)]
struct BreakingBlockMarker;

fn break_blocks(
    mut commands: Commands,
    time: Res<Time>,
    net: Res<Server>,
    items: Res<Items>,
    models: Res<Models>,
    world_map: Res<WorldMap>,
    chunk_subscriptions: Res<ChunkSubscriptions>,
    inventory_query: Query<&Inventory, With<Player>>,
    block_model_query: Query<&Transform, (With<BlockPosition>, With<Model>)>,
    mut breaking_model_query: Query<(&mut Model, &mut ModelVisibility), With<BreakingBlockMarker>>,
    mut block_update_writer: EventWriter<BlockUpdate>,
    mut mining_events: ResMut<MiningEvents>,
    mut being_broken: Local<HashMap<BlockPosition, BreakingBlock>>,
    mut rng: Local<Rng>,
) {
    let now = std::time::Instant::now();

    let blocks = Blocks::get();

    for (block_position, (player_entity, block_id, block_face, hit_position, maybe_block_entity)) in
        mining_events.drain()
    {
        let block_config = blocks.get_config(&block_id);

        let Some(hardness) = block_config.hardness else {
            // Unbreakable block
            continue;
        };

        let inventory = inventory_query.get(player_entity).unwrap();

        let tool_config = if let Some(item) = inventory.held_item_stack().item() {
            Some(items.get_config(&item.id))
        } else {
            None
        };

        let broken = if let Some(breaking_block) = being_broken.get_mut(&block_position) {
            if (now - breaking_block.prev_hit).as_secs_f32() > 0.05 {
                // The interval between two clicks needs to be short in order to be counted as
                // holding the button down.
                breaking_block.prev_hit = now;
                continue;
            }

            if breaking_block.particle_timer.finished() {
                let chunk_position = ChunkPosition::from(block_position);
                if let Some(subscribers) = chunk_subscriptions.get_subscribers(&chunk_position) {
                    if let Some(particle_effect) =
                        hit_particles(block_config, block_face, hit_position)
                    {
                        net.send_many(subscribers, particle_effect);
                    }

                    if let Some(hit_sound) = block_config.sound.hit(&mut rng) {
                        net.send_many(
                            subscribers,
                            messages::Sound {
                                position: Some(hit_position),
                                volume: 0.2,
                                speed: 0.5,
                                sound: hit_sound.to_owned(),
                            },
                        )
                    }
                }
            }

            // The timer is set to finished on the first hit to show particles immediately.
            // If we tick before checking if it is finished it will set itself to unfinished again.
            breaking_block.particle_timer.tick(time.delta());

            let (mut model, mut visibility) = breaking_model_query
                .get_mut(breaking_block.model_entity)
                .unwrap();

            let prev_progress = breaking_block.progress;

            let efficiency = if let Some(config) = tool_config {
                config.tool_efficiency(block_config)
            } else {
                1.0
            };
            breaking_block.progress +=
                (now - breaking_block.prev_hit).as_secs_f32() / hardness * efficiency;
            breaking_block.prev_hit = now;

            let progress = breaking_block.progress;

            // Ordering from high to low lets it skip stages.
            let next_texture = if prev_progress < 0.9 && progress > 0.9 {
                Some("blocks/breaking_9.png".to_owned())
            } else if prev_progress < 0.8 && progress > 0.8 {
                Some("blocks/breaking_8.png".to_owned())
            } else if prev_progress < 0.7 && progress > 0.7 {
                Some("blocks/breaking_7.png".to_owned())
            } else if prev_progress < 0.6 && progress > 0.6 {
                Some("blocks/breaking_6.png".to_owned())
            } else if prev_progress < 0.5 && progress > 0.5 {
                Some("blocks/breaking_5.png".to_owned())
            } else if prev_progress < 0.4 && progress > 0.4 {
                Some("blocks/breaking_4.png".to_owned())
            } else if prev_progress < 0.3 && progress > 0.3 {
                Some("blocks/breaking_3.png".to_owned())
            } else if prev_progress < 0.2 && progress > 0.2 {
                Some("blocks/breaking_2.png".to_owned())
            } else if prev_progress < 0.1 && progress > 0.1 {
                *visibility = ModelVisibility::Visible;
                None
            } else {
                None
            };

            if next_texture.is_some() {
                // This triggers change detection, so we do it after we determine if the texture
                // should change.
                let Model::Custom {
                    ref mut material_parallax_texture,
                    ..
                } = *model
                else {
                    unreachable!()
                };
                *material_parallax_texture = next_texture;
            }

            if progress >= 1.0 {
                true
            } else {
                continue;
            }
        } else {
            false
        };

        // When hardness is zero it will break instantly
        if broken || hardness == 0.0 {
            let chunk_position = ChunkPosition::from(block_position);
            if let Some(subscribers) = chunk_subscriptions.get_subscribers(&chunk_position) {
                let position = block_position.as_dvec3() + DVec3::splat(0.5);
                if let Some(particle_effect) = break_particles(block_config, position) {
                    net.send_many(subscribers, particle_effect);
                }

                if let Some(destroy_sound) = block_config.sound.destroy(&mut rng) {
                    net.send_many(
                        subscribers,
                        messages::Sound {
                            position: Some(position),
                            volume: 1.0,
                            speed: 1.0,
                            sound: destroy_sound.to_owned(),
                        },
                    )
                }
            }

            // TODO: Dropping a block like this is too error prone. If two systems break a block at
            // once, it will dupe. Also too much boilerplate just to drop an item, it should just
            // be:
            // block_break_events.write(BreakEvent {
            //     position: IVec3,
            //     something to signify if it should drop
            // })
            block_update_writer.write(BlockUpdate::Replace {
                position: block_position,
                block_id: blocks.get_id("air"),
                block_state: None,
                block_data: None,
            });

            let (dropped_item_id, count) = match block_config.drop(tool_config) {
                Some(drop) => drop,
                None => continue,
            };

            let item_config = items.get_config(&dropped_item_id);
            let item_stack = ItemStack::new(item_config, count);

            commands.spawn((
                DroppedItem::new(item_stack),
                Transform::from_translation(block_position.as_dvec3() + DVec3::splat(0.5)),
            ));
        } else {
            let (model, offset) = build_breaking_model(&block_config, &models);

            let model_entity = if maybe_block_entity
                .is_some_and(|e| block_model_query.get(e).is_ok())
            {
                let child = commands
                    .spawn((
                        model,
                        // The model shouldn't show until some progress has been made
                        ModelVisibility::Hidden,
                        BreakingBlockMarker,
                    ))
                    .id();
                commands
                    .entity(maybe_block_entity.unwrap())
                    .add_child(child);
                child
            } else {
                let block_state = world_map
                    .get_block_state(block_position)
                    .unwrap_or_default();
                let rotation = block_state
                    .rotation()
                    .map(|r| r.as_quat())
                    .unwrap_or_default();
                commands
                    .spawn((
                        model,
                        Transform::from_translation(block_position.as_dvec3() + offset.as_dvec3())
                            // Scale a little so it envelops the block
                            .with_scale(DVec3::splat(1.001))
                            .with_rotation(rotation),
                        // The model shouldn't show until some progress has been made
                        ModelVisibility::Hidden,
                        BreakingBlockMarker,
                    ))
                    .id()
            };

            let particle_timer = Timer::new(
                std::time::Duration::from_secs_f32(0.2),
                TimerMode::Repeating,
            );
            // Tick the timer so the first particles show up immediately
            //particle_timer.tick(std::time::Duration::from_secs(1));

            being_broken.insert(
                block_position,
                BreakingBlock {
                    model_entity,
                    progress: 0.0,
                    prev_hit: std::time::Instant::now(),
                    particle_timer,
                },
            );
        }
    }

    // Remove break progress after not being hit for 0.5 seconds.
    being_broken.retain(|_, breaking_block| {
        let remove_timout = (now - breaking_block.prev_hit).as_secs_f32() > 0.5;
        let remove_broken = breaking_block.progress >= 1.0;

        if remove_timout || remove_broken {
            // If the breaking model is the child of a block model, it will be despawned when the
            // block changes, so it will no longer be available.
            if let Ok(mut entity) = commands.get_entity(breaking_block.model_entity) {
                entity.try_despawn();
            }
            return false;
        } else {
            return true;
        }
    });
}

fn hit_particles(
    block_config: &BlockConfig,
    block_face: BlockFace,
    position: DVec3,
) -> Option<messages::ParticleEffect> {
    let Some(particle_texture) = block_config.particle_texture(block_face) else {
        return None;
    };

    let direction = block_face
        .shift_position(BlockPosition::default())
        .as_vec3();
    let spawn_offset = Vec3::select(direction.cmpeq(Vec3::ZERO), Vec3::splat(0.4), Vec3::ZERO);

    const VELOCITY: Vec3 = Vec3::new(2.5, 1.5, 2.5);
    let mut min_velocity = Vec3::select(direction.cmpeq(Vec3::ZERO), -VELOCITY, Vec3::ZERO);
    min_velocity.y = 0.0;

    let mut max_velocity = -min_velocity;
    max_velocity += direction * 2.0;
    max_velocity.y = max_velocity.y.max(VELOCITY.y);

    // Need to offset so the particle's aabb won't be inside the block
    let block_face_offset = block_face
        .shift_position(BlockPosition::default())
        .as_dvec3()
        * 0.15;

    Some(messages::ParticleEffect::Explosion {
        position: position + block_face_offset,
        spawn_offset,
        size_range: (0.1, 0.2),
        min_velocity,
        max_velocity,
        texture: Some(particle_texture.to_owned()),
        color: block_config.particle_color(),
        lifetime: (0.3, 1.0),
        count: 4,
    })
}

fn break_particles(
    block_config: &BlockConfig,
    position: DVec3,
) -> Option<messages::ParticleEffect> {
    let Some(particle_texture) = block_config.particle_texture(BlockFace::Bottom) else {
        return None;
    };

    const VELOCITY: Vec3 = Vec3::new(7.0, 5.0, 7.0);

    Some(messages::ParticleEffect::Explosion {
        position,
        spawn_offset: Vec3::splat(0.2),
        size_range: (0.2, 0.3),
        min_velocity: -VELOCITY,
        max_velocity: VELOCITY,
        texture: Some(particle_texture.to_owned()),
        color: block_config.particle_color(),
        lifetime: (0.3, 1.0),
        count: 20,
    })
}

fn build_breaking_model(block_config: &BlockConfig, models: &Models) -> (Model, Vec3) {
    if let Some(model_id) = block_config.model {
        let model = models.get_by_id(model_id);
        let mut mesh_vertices = Vec::new();
        let mut mesh_uvs = Vec::new();
        let mut mesh_normals = Vec::new();
        let mut mesh_indices = Vec::new();
        let mut current_index = 0;

        for mesh in model.meshes.iter() {
            mesh_vertices.extend(
                mesh.vertices
                    .iter()
                    .cloned()
                    .zip(&mesh.normals)
                    // Since scaling isn't possible, we simply push the mesh a little in the
                    // direction of the normal to have it overlay the mesh of the thing that is
                    // breaking.
                    .map(|(v, n)| (Vec3::from_array(v) + Vec3::from_array(*n) * 0.001).to_array()),
            );
            mesh_normals.extend(&mesh.normals);
            mesh_indices.extend(mesh.indices.iter().map(|index| index + current_index));
            current_index += mesh.indices.len() as u32;

            for (k, uv_quad) in mesh.uvs.chunks_exact(4).enumerate() {
                let mut min = [f32::MAX, f32::MAX];
                let mut max = [f32::MIN, f32::MIN];
                for uv in uv_quad {
                    min[0] = uv[0].min(min[0]);
                    max[0] = uv[0].max(max[0]);

                    min[1] = uv[1].min(min[1]);
                    max[1] = uv[1].max(max[1]);
                }

                let mut corners = [0; 4];
                let bottom_left = [min[0], max[1]];
                let top_right = [max[0], min[1]];
                for (j, uv) in uv_quad.iter().enumerate() {
                    corners[j] = if *uv == min {
                        0
                    } else if *uv == bottom_left {
                        1
                    } else if *uv == max {
                        2
                    } else if *uv == top_right {
                        3
                    } else {
                        unreachable!()
                    };
                }

                let vertices = &mesh.vertices[k * 4..k * 4 + 4];

                let top_left = corners.iter().position(|&c| c == 0).unwrap();
                let top_right = corners.iter().position(|&c| c == 3).unwrap();
                let bottom_left = corners.iter().position(|&c| c == 1).unwrap();
                let width = Vec3::from_array(vertices[top_left])
                    .distance(Vec3::from_array(vertices[top_right]));
                let height = Vec3::from_array(vertices[top_left])
                    .distance(Vec3::from_array(vertices[bottom_left]));
                let width_offset = (1.0 - width.min(1.0)) / 2.0;
                let height_offset = (1.0 - height.min(1.0)) / 2.0;

                let mut uvs = [[0.0; 2]; 4];
                for (index, corner) in corners.iter().enumerate() {
                    uvs[index] = match corner {
                        0 => [width_offset, height_offset],
                        1 => [width_offset, 1.0 - height_offset],
                        2 => [1.0 - width_offset, 1.0 - height_offset],
                        3 => [1.0 - width_offset, height_offset],
                        _ => unreachable!(),
                    };
                }

                mesh_uvs.extend(uvs);
            }
            // TODO: This should not break. It messes up consecutive meshes, vertices all over the
            // place.
            break;
        }

        (
            Model::Custom {
                mesh_indices,
                mesh_vertices,
                mesh_normals,
                mesh_uvs: Some(mesh_uvs),
                material_color_texture: None,
                material_parallax_texture: Some("blocks/breaking_1.png".to_owned()),
                material_alpha_mode: 2,
                material_alpha_cutoff: 0.0,
                material_double_sided: false,
                collider: None,
            },
            Vec3::ZERO,
        )
    } else if let Some(quads) = &block_config.quads {
        let mut mesh_vertices = Vec::new();
        let mut mesh_uvs = Vec::new();
        let mut mesh_normals = Vec::new();
        let mut mesh_indices = Vec::new();

        for (i, quad) in quads.iter().enumerate() {
            let normals = [
                (Vec3::from_array(quad.vertices[1]) - Vec3::from_array(quad.vertices[0]))
                    .cross(Vec3::from_array(quad.vertices[2]) - Vec3::from_array(quad.vertices[1]))
                    .to_array(),
                (Vec3::from_array(quad.vertices[3]) - Vec3::from_array(quad.vertices[1]))
                    .cross(Vec3::from_array(quad.vertices[2]) - Vec3::from_array(quad.vertices[1]))
                    .to_array(),
            ];

            mesh_vertices.extend(quad.vertices.map(|v| {
                [
                    v[0] - 0.5 + normals[0][0] * 0.0001,
                    v[1] - 0.5 + normals[0][1] * 0.0001,
                    v[2] - 0.5 + normals[0][2] * 0.0001,
                ]
            }));

            mesh_normals.extend([normals[0], normals[0], normals[1], normals[1]]);

            const INDICES: [u32; 6] = [0, 1, 2, 2, 1, 3];
            mesh_indices.extend(INDICES.iter().map(|x| x + 4 * i as u32));

            let normal = Vec3::from(normals[0]);
            let normal_max = normal.abs().cmpeq(Vec3::splat(normal.abs().max_element()));

            let mut uvs: [[f32; 2]; 4] = default();
            for (i, vertex) in quad.vertices.into_iter().enumerate() {
                let uv = if normal_max.x {
                    Vec3::from_array(vertex).zy()
                } else if normal_max.y {
                    Vec3::from_array(vertex).xz()
                } else {
                    Vec3::from_array(vertex).xy()
                };

                if i == 0 {
                    uvs[0] = [uv.x - uv.x.floor(), uv.y - uv.y.floor()];
                } else if i == 1 {
                    uvs[1] = [
                        // This is just the fraction, but instead of using `f32::fract`
                        // we do this so it's inversed for negative numbers. e.g. -0.6
                        // yields 0.4, which is what we want because that is the distance
                        // from -1.0 to -0.6
                        uv.x - uv.x.floor(),
                        // Since this is on the high side of the range extracting the fract is harder since it
                        // can be a whole number. e.g a position of 1.0 should give a fraction of 1.0, not 0.0.
                        uv.y - (uv.y.ceil() - 1.0),
                    ];
                } else if i == 2 {
                    uvs[2] = [uv.x - (uv.x.ceil() - 1.0), uv.y - uv.y.floor()];
                } else if i == 3 {
                    uvs[3] = [uv.x - (uv.x.ceil() - 1.0), uv.y - (uv.y.ceil() - 1.0)];
                }
            }

            mesh_uvs.extend(uvs);
        }

        (
            Model::Custom {
                mesh_indices,
                mesh_vertices,
                mesh_normals,
                mesh_uvs: Some(mesh_uvs),
                material_color_texture: None,
                material_parallax_texture: Some("blocks/breaking_1.png".to_owned()),
                material_alpha_mode: 2,
                material_alpha_cutoff: 0.0,
                material_double_sided: false,
                collider: None,
            },
            Vec3::splat(0.5),
        )
    } else {
        let mesh_vertices = vec![
            // Top
            [-0.5, 0.5, -0.5],
            [-0.5, 0.5, 0.5],
            [0.5, 0.5, -0.5],
            [0.5, 0.5, 0.5],
            // Back
            [0.5, 0.5, -0.5],
            [0.5, -0.5, -0.5],
            [-0.5, 0.5, -0.5],
            [-0.5, -0.5, -0.5],
            // Left
            [-0.5, 0.5, -0.5],
            [-0.5, -0.5, -0.5],
            [-0.5, 0.5, 0.5],
            [-0.5, -0.5, 0.5],
            // Right
            [0.5, 0.5, 0.5],
            [0.5, -0.5, 0.5],
            [0.5, 0.5, -0.5],
            [0.5, -0.5, -0.5],
            // Front
            [-0.5, 0.5, 0.5],
            [-0.5, -0.5, 0.5],
            [0.5, 0.5, 0.5],
            [0.5, -0.5, 0.5],
            // Bottom
            [-0.5, -0.5, 0.5],
            [-0.5, -0.5, -0.5],
            [0.5, -0.5, 0.5],
            [0.5, -0.5, -0.5],
        ];

        let mesh_normals = vec![
            // Top
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            // Back
            [0.0, 0.0, -1.0],
            [0.0, 0.0, -1.0],
            [0.0, 0.0, -1.0],
            [0.0, 0.0, -1.0],
            // Left
            [-1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            // Right
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            // Front
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            // Bottom
            [0.0, -1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, -1.0, 0.0],
        ];

        const UVS: [[f32; 2]; 4] = [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]];
        let mut mesh_uvs = Vec::new();
        for _ in 0..6 {
            mesh_uvs.extend(UVS);
        }

        const INDICES: [u32; 6] = [0, 1, 2, 2, 1, 3];
        let mut mesh_indices = Vec::new();
        for i in 0..6 {
            mesh_indices.extend(INDICES.iter().map(|x| x + 4 * i));
        }

        (
            Model::Custom {
                mesh_indices,
                mesh_vertices,
                mesh_normals,
                mesh_uvs: Some(mesh_uvs),
                material_color_texture: None,
                material_parallax_texture: Some("blocks/breaking_1.png".to_owned()),
                material_alpha_mode: 2,
                material_alpha_cutoff: 0.0,
                material_double_sided: false,
                collider: None,
            },
            Vec3::splat(0.5),
        )
    }
}

fn handle_right_clicks(
    net: Res<Server>,
    world_map: Res<WorldMap>,
    items: Res<Items>,
    item_registry: Res<ItemRegistry>,
    model_map: Res<ModelMap>,
    chunk_subscriptions: Res<ChunkSubscriptions>,
    model_query: Query<(&Collider, &GlobalTransform), (With<Model>, Without<BlockPosition>)>,
    mut player_query: Query<(&mut Inventory, &Targets, &Camera), With<Player>>,
    mut item_use_query: Query<&mut ItemUses>,
    mut hand_interaction_query: Query<&mut HandInteractions>,
    mut block_update_writer: EventWriter<BlockUpdate>,
    mut clicks: EventReader<NetworkMessage<messages::RightClick>>,
    mut rng: Local<Rng>,
) {
    // TODO: ActionOrder currently does nothing, but there needs to be some system for deviating
    // from the set order. Like if you hold shift, placing blocks should take precedence over
    // interacting. And there's a bunch of stuff like this where you want to do something else
    // depending on some condition.
    enum ActionOrder {
        Interact,
        PlaceBlock,
        UseItem,
    }

    for right_click in clicks.read() {
        let (mut inventory, targets, camera) =
            player_query.get_mut(right_click.player_entity).unwrap();

        let mut action = ActionOrder::Interact;

        'outer: loop {
            match action {
                ActionOrder::Interact => {
                    for target in targets.iter() {
                        let Some(entity) = target.entity() else {
                            continue;
                        };

                        if let Ok(mut interactions) = hand_interaction_query.get_mut(entity) {
                            interactions.push(right_click.player_entity);
                            break 'outer;
                        }
                    }

                    action = ActionOrder::PlaceBlock;
                }
                ActionOrder::PlaceBlock => {
                    let blocks = Blocks::get();

                    let Some(Target::Block {
                        block_position,
                        block_id,
                        block_face,
                        ..
                    }) = targets
                        .get_first_block(|block_id| blocks.get_config(block_id).hardness.is_some())
                    else {
                        action = ActionOrder::UseItem;
                        continue;
                    };

                    let blocks = Blocks::get();
                    let equipped_item_stack = inventory.held_item_stack_mut();

                    if let Some((block_id, replaced_block_position)) = block_placement(
                        &equipped_item_stack,
                        *block_id,
                        *block_face,
                        *block_position,
                        &items,
                        &blocks,
                        &world_map,
                    ) {
                        let block_config = blocks.get_config(&block_id);
                        let block_state = block_config.placement_rotation(*block_face, camera);

                        let replaced_collider = Collider::Aabb(Aabb {
                            center: replaced_block_position.as_dvec3(),
                            half_extents: DVec3::splat(0.5),
                        });

                        // Check that there aren't any entities in the way of the new block
                        let chunk_position = ChunkPosition::from(replaced_block_position);
                        if let Some(entities) = model_map.get_entities(&chunk_position) {
                            for (collider, global_transform) in model_query.iter_many(entities) {
                                let collider =
                                    collider.transform(&global_transform.compute_transform());

                                if collider.intersection(&replaced_collider).is_some() {
                                    continue;
                                }
                            }
                        }

                        equipped_item_stack.take(1);

                        if let Some(subscribers) =
                            chunk_subscriptions.get_subscribers(&chunk_position)
                        {
                            let position = block_position.as_dvec3() + DVec3::splat(0.5);

                            if let Some(place_sound) = block_config.sound.place(&mut rng) {
                                net.send_many(
                                    subscribers,
                                    messages::Sound {
                                        position: Some(position),
                                        volume: 1.0,
                                        speed: 1.0,
                                        sound: place_sound.to_owned(),
                                    },
                                )
                            }
                        }

                        block_update_writer.write(BlockUpdate::Replace {
                            position: replaced_block_position,
                            block_id,
                            block_state,
                            block_data: None,
                        });

                        break;
                    } else {
                        action = ActionOrder::UseItem;
                    }
                }
                ActionOrder::UseItem => {
                    // If nothing else was done, we try to use the item
                    let equipped_item_stack = inventory.held_item_stack_mut();

                    let Some(item) = equipped_item_stack.item() else {
                        break;
                    };

                    if let Some(item_use_entity) = item_registry.get(&item.id) {
                        let mut uses = item_use_query.get_mut(*item_use_entity).unwrap();
                        uses.push(right_click.player_entity);
                    }

                    break;
                }
            }
        }
    }
}

fn block_placement(
    equipped_item_stack: &ItemStack,
    block_id: BlockId,
    block_face: BlockFace,
    block_position: BlockPosition,
    items: &Items,
    blocks: &Blocks,
    world_map: &WorldMap,
) -> Option<(BlockId, BlockPosition)> {
    let against_block = blocks.get_config(&block_id);

    if !against_block.is_solid() {
        return None;
    }

    let Some(item) = equipped_item_stack.item() else {
        // No item equipped, can't place block
        return None;
    };

    let item_config = items.get_config(&item.id);

    let Some(new_block_id) = item_config.block else {
        // The item isn't bound to a placeable block
        return None;
    };

    if !blocks.get_config(&new_block_id).is_placeable(block_face) {
        return None;
    }

    let replaced_block_position = if against_block.replaceable {
        // Some blocks, like grass, can be replaced instead of placing the new
        // block adjacently to it.
        block_position
    } else if let Some(block_id) = world_map.get_block(block_face.shift_position(block_position)) {
        if !blocks.get_config(&block_id).replaceable {
            return None;
        }
        block_face.shift_position(block_position)
    } else {
        return None;
    };

    return Some((new_block_id, replaced_block_position));
}
