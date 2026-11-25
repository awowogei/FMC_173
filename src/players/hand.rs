use std::collections::{HashMap, HashSet};

use fmc::{
    audio::{AudioPlayer, Sound},
    blocks::{BlockFace, BlockId, BlockPosition, BlockRotation, BlockState},
    items::{ItemId, ItemStack},
    math::DVec3,
    models::{AnimationPlayer, Model, ModelConfig, Models},
    networking::NetworkMessage,
    particle_effects::{ParticleEffect, ParticleSpawner},
    physics::{Collider, GRAVITY, shapes::Aabb},
    players::{Camera, Player, Target, Targets},
    prelude::*,
    protocol::messages,
    random::Rng,
    visibility::{EntityMap, Visibility},
    world::{BlockUpdate, WorldMap, chunk::ChunkPosition},
};

use crate::{
    items::{DroppedItem, ItemEvents, ItemSystems, Items},
    players::Inventory,
    world::blocks::{BlockConfig, Blocks},
};

pub struct HandPlugin;
impl Plugin for HandPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<BlockHit>().add_systems(
            Update,
            (
                handle_left_clicks.in_set(HandSystems),
                handle_right_clicks.in_set(ItemSystems).in_set(HandSystems),
                block_mining.after(handle_left_clicks),
            ),
        );
    }
}

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub struct HandSystems;

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
    mut clicks: MessageReader<NetworkMessage<messages::LeftClick>>,
    models: Res<Models>,
    blocks: Res<Blocks>,
    mut player_query: Query<
        (&Targets, &Camera, &GlobalTransform, &mut AnimationPlayer),
        With<Player>,
    >,
    mut hittable_entities: Query<(&mut HandHits, Option<&Visibility>)>,
    mut block_hits: MessageWriter<BlockHit>,
    mut click_tracker: Local<HashSet<Entity>>,
) {
    for (mut hand_hits, _) in hittable_entities.iter_mut() {
        if !hand_hits.player_entities.is_empty() {
            hand_hits.player_entities.clear();
        }
    }

    for click in clicks.read() {
        let (targets, camera, transform, mut animation_player) =
            player_query.get_mut(click.player_entity).unwrap();

        let camera_position = transform.translation() + camera.translation;

        let model = models.get_config_by_name("player").unwrap();
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
                    let block_config = blocks.get_config(block_id);

                    if block_config.hardness.is_some() {
                        let hit_position = camera_position + camera.forward() * *distance;
                        block_hits.write(BlockHit {
                            player_entity: click.player_entity,
                            block_position: *block_position,
                            block_id: *block_id,
                            block_face: *block_face,
                            hit_position,
                            block_entity: *entity,
                        });

                        break;
                    }
                }
                Target::Entity { entity, .. } if first_click => {
                    if let Ok((mut hits, maybe_visibility)) = hittable_entities.get_mut(*entity) {
                        if matches!(maybe_visibility, Some(Visibility::Hidden)) {
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

#[derive(Message)]
struct BlockHit {
    player_entity: Entity,
    block_position: BlockPosition,
    block_id: BlockId,
    block_face: BlockFace,
    hit_position: DVec3,
    block_entity: Option<Entity>,
}

#[derive(Component)]
pub struct Miner {
    target: Option<BlockPosition>,
    prev_hit: std::time::Instant,
    particle_timer: Timer,
}

impl Default for Miner {
    fn default() -> Self {
        Self {
            target: None,
            prev_hit: std::time::Instant::now(),
            particle_timer: Timer::from_seconds(0.2, TimerMode::Repeating),
        }
    }
}

struct MiningProgress {
    block_position: BlockPosition,
    block_id: BlockId,
    block_entity: Option<Entity>,
    progress: f32,
    tool: Option<ItemId>,
}

// Keeps the state of how far along a block is to breaking
#[derive(Debug)]
struct BreakingBlock {
    model_entity: Option<Entity>,
    progress: f32,
    last_progress: std::time::Instant,
}

#[derive(Default)]
struct BreakingState {
    blocks: HashMap<BlockPosition, BreakingBlock>,
    progress: Vec<MiningProgress>,
    rng: Rng,
}

#[derive(Component)]
struct BreakingBlockMarker;

fn block_mining(
    mut commands: Commands,
    time: Res<Time>,
    blocks: Res<Blocks>,
    audio: AudioPlayer,
    particles: ParticleSpawner,
    items: Res<Items>,
    models: Res<Models>,
    world_map: Res<WorldMap>,
    mut miner_query: Query<(&Inventory, &mut Miner), With<Player>>,
    block_model_query: Query<&Transform, (With<BlockPosition>, With<Model>)>,
    mut breaking_model_query: Query<&mut Model, With<BreakingBlockMarker>>,
    mut block_update_writer: MessageWriter<BlockUpdate>,
    mut block_hits: MessageReader<BlockHit>,
    mut state: Local<BreakingState>,
) {
    let now = std::time::Instant::now();
    let BreakingState {
        blocks: breaking_blocks,
        progress: mining_progress,
        rng,
    } = &mut *state;

    for hit in block_hits.read() {
        let Ok((inventory, mut miner)) = miner_query.get_mut(hit.player_entity) else {
            continue;
        };

        if miner.target == Some(hit.block_position) && miner.prev_hit == now {
            continue;
        }

        let block_config = blocks.get_config(&hit.block_id);

        let Some(hardness) = block_config.hardness else {
            // Unbreakable block
            continue;
        };

        let tool = inventory.held_item_stack().item().map(|item| item.id);
        let tool_config = tool.map(|id| items.get_config(&id));

        // The interval between two clicks needs to be short in order to be counted as
        // holding the button down.
        let continuing = miner.target == Some(hit.block_position)
            && (now - miner.prev_hit).as_secs_f32() <= 0.05;

        let progress = if hardness == 0.0 {
            1.0
        } else if continuing {
            let efficiency = match tool_config.and_then(|config| config.tool.as_ref()) {
                Some(tool) if block_config.tools.contains(&tool.name) => tool.efficiency,
                _ => 1.0,
            };
            (now - miner.prev_hit).as_secs_f32() / hardness * efficiency
        } else {
            0.0
        };

        if continuing {
            if miner.particle_timer.is_finished() {
                if let Some(texture) = block_config.particle_texture(hit.block_face) {
                    // Need to offset so the particle's aabb won't be inside the block
                    let block_face_offset = hit
                        .block_face
                        .shift_position(BlockPosition::default())
                        .as_dvec3()
                        * 0.15;

                    particles.spawn(
                        hit.hit_position + block_face_offset,
                        ParticleEffect::new("block_hit", texture)
                            // TODO: Silly quat conversion
                            .with_rotation(hit.block_face.to_quat().as_quat())
                            .with_color(block_config.particle_color().unwrap_or(Vec4::ONE)),
                    );
                }

                if let Some(hit_sound) = block_config.sound.hit(rng) {
                    audio.play_at(
                        hit.hit_position,
                        Sound::new(hit_sound).with_volume(0.2).with_speed(0.5),
                    );
                }
            }
            miner.particle_timer.tick(time.delta());
        } else {
            miner.particle_timer.reset();
        }

        miner.target = Some(hit.block_position);
        miner.prev_hit = now;

        mining_progress.push(MiningProgress {
            block_position: hit.block_position,
            block_id: hit.block_id,
            block_entity: hit.block_entity,
            progress,
            tool,
        });
    }

    for mined in mining_progress.drain(..) {
        let block_config = blocks.get_config(&mined.block_id);

        let breaking_block = breaking_blocks
            .entry(mined.block_position)
            .or_insert(BreakingBlock {
                model_entity: None,
                progress: 0.0,
                last_progress: now,
            });

        if breaking_block.progress >= 1.0 {
            continue;
        }

        let prev_stage = (breaking_block.progress * 10.0) as u32;
        breaking_block.progress += mined.progress;
        breaking_block.last_progress = now;

        if breaking_block.progress >= 1.0 {
            let position = mined.block_position.as_dvec3() + DVec3::splat(0.5);

            if let Some(texture) = block_config.particle_texture(BlockFace::Bottom) {
                particles.spawn(
                    position,
                    ParticleEffect::new("block_break", texture)
                        .with_color(block_config.particle_color().unwrap_or(Vec4::ONE)),
                );
            }

            if let Some(destroy_sound) = block_config.sound.destroy(rng) {
                audio.play_at(position, Sound::new(destroy_sound));
            }

            // TODO: Dropping a block like this is too error prone. If two systems break a block at
            // once, it will dupe. Also too much boilerplate just to drop an item, it should just
            // be:
            // block_break_events.write(BreakEvent {
            //     position: IVec3,
            //     something to signify if it should drop
            // })
            block_update_writer.write(BlockUpdate::Replace {
                position: mined.block_position,
                block_id: blocks.get_id("air"),
                block_state: None,
                block_data: None,
            });

            let tool_config = mined.tool.map(|id| items.get_config(&id));
            if let Some(item_stack) = block_config.drop(tool_config, rng) {
                commands.spawn((
                    DroppedItem::new(item_stack),
                    Transform::from_translation(position),
                ));
            }

            continue;
        }

        let stage = (breaking_block.progress * 10.0) as u32;
        if stage == prev_stage {
            continue;
        }

        let texture = format!("blocks/breaking_{}.png", stage);

        match breaking_block.model_entity {
            Some(model_entity) => {
                if let Ok(mut model) = breaking_model_query.get_mut(model_entity)
                    && let Model::Custom {
                        ref mut material_parallax_texture,
                        ..
                    } = *model
                {
                    *material_parallax_texture = Some(texture);
                }
            }
            None => {
                breaking_block.model_entity = Some(spawn_breaking_model(
                    &mut commands,
                    block_config,
                    &models,
                    &world_map,
                    &block_model_query,
                    mined.block_position,
                    mined.block_entity,
                    texture,
                ));
            }
        }
    }

    breaking_blocks.retain(|_, breaking_block| {
        // Remove break progress after not being hit for 0.5 seconds.
        let timed_out = (now - breaking_block.last_progress).as_secs_f32() > 0.5;
        let broken = breaking_block.progress >= 1.0;
        let remove = timed_out || broken;

        // If the breaking model is the child of a block model, it will be despawned when the
        // block changes, so it will no longer be available.
        if remove
            && let Some(model_entity) = breaking_block.model_entity
            && let Ok(mut entity) = commands.get_entity(model_entity)
        {
            entity.try_despawn();
        }

        !remove
    });
}

// Broken out into a function to ease up on the syntax clutter
fn spawn_breaking_model(
    commands: &mut Commands,
    block_config: &BlockConfig,
    models: &Models,
    world_map: &WorldMap,
    block_model_query: &Query<&Transform, (With<BlockPosition>, With<Model>)>,
    block_position: BlockPosition,
    block_entity: Option<Entity>,
    texture: String,
) -> Entity {
    let (model, offset) = build_breaking_model(block_config, models, texture);

    if let Some(block_entity) = block_entity.filter(|e| block_model_query.get(*e).is_ok()) {
        let child = commands.spawn((model, BreakingBlockMarker)).id();
        commands.entity(block_entity).add_child(child);
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
                BreakingBlockMarker,
            ))
            .id()
    }
}

fn build_breaking_model(
    block_config: &BlockConfig,
    models: &Models,
    texture: String,
) -> (Model, Vec3) {
    let (vertices, normals, uvs, indices, offset) = if let Some(model_id) = block_config.model {
        let model = models.get_config(&model_id);
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
            current_index += mesh.vertices.len() as u32;

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
        }

        (
            mesh_vertices,
            mesh_normals,
            mesh_uvs,
            mesh_indices,
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
            mesh_vertices,
            mesh_normals,
            mesh_uvs,
            mesh_indices,
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
            mesh_vertices,
            mesh_normals,
            mesh_uvs,
            mesh_indices,
            Vec3::splat(0.5),
        )
    };

    (
        Model::Custom {
            mesh_indices: indices,
            mesh_vertices: vertices,
            mesh_normals: normals,
            mesh_uvs: Some(uvs),
            material_color_texture: None,
            material_parallax_texture: Some(texture),
            material_alpha_mode: 2,
            material_alpha_cutoff: 0.0,
            material_double_sided: false,
        },
        offset,
    )
}

fn handle_right_clicks(
    audio: AudioPlayer,
    world_map: Res<WorldMap>,
    blocks: Res<Blocks>,
    items: Res<Items>,
    entity_map: Res<EntityMap>,
    model_query: Query<(&Collider, &GlobalTransform), (With<Model>, Without<BlockPosition>)>,
    mut player_query: Query<(&mut Inventory, &Targets, &Camera), With<Player>>,
    mut item_events_query: Query<&mut ItemEvents>,
    mut hand_interaction_query: Query<&mut HandInteractions>,
    mut block_update_writer: MessageWriter<BlockUpdate>,
    mut clicks: MessageReader<NetworkMessage<messages::RightClick>>,
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
                    let Some(Target::Block {
                        block_position,
                        block_id,
                        block_face,
                        ..
                    }) = targets.get_first_block(|block_id| blocks.get_config(block_id).is_solid())
                    else {
                        action = ActionOrder::UseItem;
                        continue;
                    };

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
                        let block_state = block_config
                            .placement
                            .placement_rotation(*block_face, camera);

                        let chunk_position = ChunkPosition::from(replaced_block_position);

                        let rotation = block_state
                            .map(BlockRotation::from)
                            .map(BlockRotation::as_quat)
                            .unwrap_or_default();

                        let replaced_block_transform = Transform {
                            translation: block_position.as_dvec3() + DVec3::splat(0.5),
                            rotation,
                            ..default()
                        };

                        for (model_collider, global_transform) in
                            model_query.iter_many(entity_map.iter_entities(&chunk_position))
                        {
                            if model_collider
                                .intersection(
                                    &global_transform.compute_transform(),
                                    &replaced_block_transform,
                                    &block_config.collider,
                                )
                                .is_some()
                            {
                                continue;
                            }
                        }

                        equipped_item_stack.pop();

                        if let Some(place_sound) = block_config.sound.place(&mut rng) {
                            let position = block_position.as_dvec3() + DVec3::splat(0.5);
                            audio.play_at(position, Sound::new(place_sound));
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

                    if let Some(handler_entity) = items.get_event_handler(item.id) {
                        let mut item_events = item_events_query.get_mut(handler_entity).unwrap();
                        item_events.push(right_click.player_entity);
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

    if !blocks
        .get_config(&new_block_id)
        .placement
        .is_placeable(block_face)
    {
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
