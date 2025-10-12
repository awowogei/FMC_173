use std::{f32::consts::FRAC_PI_2, ops::Mul, time::Duration};

use fmc::{
    bevy::math::{DQuat, DVec2, DVec3},
    blocks::{BlockPosition, Blocks},
    items::Items,
    models::{Model, ModelColor, ModelVisibility, Models},
    networking::Server,
    physics::Physics,
    players::{Camera, Player},
    prelude::*,
    protocol::messages,
    random::{Rng, UniformDistribution},
    world::{
        ChunkSubscriptions, Surface, WorldMap,
        chunk::{Chunk, ChunkPosition},
    },
};
use serde::{Deserialize, Serialize};

use crate::{
    players::{HandHits, HandSystems, Inventory},
    skybox::Clock,
};

pub mod cow;
pub mod duck;
mod pathfinding;
pub mod zombie;

pub struct MobsPlugin;
impl Plugin for MobsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Mobs::default())
            .insert_resource(RandomMobs::default())
            .add_message::<MobDamageEvent>()
            .add_plugins(duck::DuckPlugin)
            .add_plugins(zombie::ZombiePlugin)
            .add_plugins(cow::CowPlugin)
            .add_systems(
                Update,
                (
                    sync_mob_caps,
                    spawn_hostile_random_mobs,
                    spawn_friendly_random_mobs,
                    despawn_mobs,
                    handle_hand_hits.after(HandSystems),
                    damage_mobs,
                    play_random_sound,
                    look_around,
                ),
            );
    }
}

pub type MobId = usize;

pub struct MobConfig {
    pub spawn_function: Box<dyn Fn(&mut EntityCommands) + Send + Sync + 'static>,
    pub sounds: MobSoundCollection,
}

#[derive(Resource, Default)]
pub struct Mobs {
    mobs: Vec<MobConfig>,
}

impl Mobs {
    pub fn add_mob(&mut self, mob_config: MobConfig) -> MobId {
        let id = self.mobs.len();
        self.mobs.push(mob_config);
        id
    }

    pub fn get_config(&self, mob_id: MobId) -> &MobConfig {
        &self.mobs[mob_id]
    }
}

// A *loose* cap on how many mobs can be spawned near a player. Each player has its own MobCap.
// When two players move within the simulation distance of each other the maximum of their caps are
// computed and applied to both.
#[derive(Component, Default, Clone, Copy)]
pub struct MobCap {
    friendly: u32,
    hostile: u32,
}

impl MobCap {
    const FRIENDLY_CAPACITY: u32 = 12;
    const HOSTILE_CAPACITY: u32 = 16;

    fn at_hostile_capacity(&self) -> bool {
        self.hostile >= Self::HOSTILE_CAPACITY
    }

    fn at_friendly_capacity(&self) -> bool {
        self.friendly >= Self::FRIENDLY_CAPACITY
    }
}

// TODO: This should probably be within some simulation distance and not render distance
//
// When players get within render distance of each other, their mob caps are synced so as to not
// spawn double the mobs when they are close to each other.
fn sync_mob_caps(
    chunk_subscriptions: Res<ChunkSubscriptions>,
    mut mob_caps: Query<&mut MobCap>,
    chunk_positions: Query<&ChunkPosition, (With<Player>, Changed<ChunkPosition>)>,
) {
    for chunk_position in chunk_positions.iter() {
        let Some(subscribers) = chunk_subscriptions.get_subscribers(&chunk_position) else {
            continue;
        };

        if subscribers.len() == 1 {
            continue;
        }

        let mut max = MobCap::default();
        for player_cap in mob_caps.iter_many(subscribers) {
            max.friendly = player_cap.friendly.max(max.friendly);
            max.hostile = player_cap.hostile.max(max.hostile);
        }

        for player_entity in subscribers {
            let mut mob_cap = mob_caps.get_mut(*player_entity).unwrap();
            *mob_cap = max;
        }
    }
}

#[derive(Component)]
enum RandomMobType {
    Hostile,
    Friendly,
}

#[derive(Resource, Default)]
pub struct RandomMobs {
    hostile: Vec<(u32, MobId)>,
    friendly: Vec<(u32, MobId)>,
}

impl RandomMobs {
    fn add_hostile(&mut self, count: u32, mob_id: MobId) {
        self.hostile.push((count, mob_id));
    }

    fn add_friendly(&mut self, count: u32, mob_id: MobId) {
        self.friendly.push((count, mob_id));
    }

    fn choose_friendly(&self, rng: &mut Rng) -> (u32, MobId) {
        let index = rng.next_usize() % self.friendly.len();
        self.friendly[index]
    }

    fn choose_hostile(&self, rng: &mut Rng) -> (u32, MobId) {
        let index = rng.next_usize() % self.hostile.len();
        self.hostile[index]
    }
}

fn spawn_friendly_random_mobs(
    mut commands: Commands,
    world_map: Res<WorldMap>,
    mobs: Res<Mobs>,
    random_mobs: Res<RandomMobs>,
    mut player_caps: Query<(&mut MobCap, &ChunkPosition)>,
    mut rng: Local<Rng>,
) {
    'outer: for (mut mob_cap, chunk_position) in player_caps.iter_mut() {
        if mob_cap.at_friendly_capacity() {
            continue;
        }

        // Choose a random chunk around the player
        let radius = 5; // actual radius is 4, modulo yields 0..radius-1
        let x = rng.next_i32() % radius * Chunk::SIZE as i32;
        let y = rng.next_i32() % radius * Chunk::SIZE as i32;
        let z = rng.next_i32() % radius * Chunk::SIZE as i32;
        let spawn_chunk = *chunk_position + ChunkPosition::new(x, y, z);

        let Some(chunk) = world_map.get_chunk(&spawn_chunk) else {
            continue;
        };

        let blocks = Blocks::get();
        let grass = blocks.get_id("grass");
        let stone = blocks.get_id("stone");
        let air = blocks.get_id("air");
        let surface_blocks = [grass, stone];
        let surface = Surface::new(chunk, &surface_blocks, air);

        let (group_size, mob_id) = random_mobs.choose_friendly(&mut rng);

        let mob_config = mobs.get_config(mob_id);

        for _ in 0..group_size {
            let x = rng.next_usize() % Chunk::SIZE;
            let z = rng.next_usize() % Chunk::SIZE;
            let mut spawn_position =
                BlockPosition::from(spawn_chunk) + BlockPosition::new(x as i32, 0, z as i32);

            let Some((y, _)) = surface[[x, z]] else {
                continue 'outer;
            };
            spawn_position.y += y as i32;

            let mut entity_commands = commands.spawn((
                Mob { id: mob_id },
                RandomMobType::Friendly,
                Transform::from_translation(spawn_position.as_dvec3() + DVec3::new(0.5, 1.0, 0.5)),
            ));

            (mob_config.spawn_function)(&mut entity_commands);

            mob_cap.friendly += 1;

            if mob_cap.at_friendly_capacity() {
                continue 'outer;
            }

            spawn_position.x = rng.next_i32().abs() % Chunk::SIZE as i32;
            spawn_position.z = rng.next_i32().abs() % Chunk::SIZE as i32;
        }
    }
}

fn spawn_hostile_random_mobs(
    mut commands: Commands,
    world_map: Res<WorldMap>,
    mobs: Res<Mobs>,
    clock: Res<Clock>,
    random_mobs: Res<RandomMobs>,
    mut player_caps: Query<(&mut MobCap, &ChunkPosition)>,
    mut rng: Local<Rng>,
) {
    'outer: for (mut mob_cap, chunk_position) in player_caps.iter_mut() {
        if mob_cap.at_hostile_capacity() {
            continue;
        }

        // Hostile mobs are spawned in the chunks that are 2 chunks away from the chunk the player is in.
        // This ensures no mob can be spawned directly in front of the player
        let face = rng.next_usize() % 6;
        let range = UniformDistribution::<i32>::new(-2, 2);
        let offset = match face {
            0 => IVec3::new(range.sample(&mut rng), 2, range.sample(&mut rng)),
            1 => IVec3::new(range.sample(&mut rng), -2, range.sample(&mut rng)),
            2 => IVec3::new(2, range.sample(&mut rng), range.sample(&mut rng)),
            3 => IVec3::new(-2, range.sample(&mut rng), range.sample(&mut rng)),
            4 => IVec3::new(range.sample(&mut rng), range.sample(&mut rng), 2),
            5 => IVec3::new(range.sample(&mut rng), range.sample(&mut rng), -2),
            _ => unreachable!(),
        };
        let spawn_chunk = *chunk_position + ChunkPosition::from(offset * Chunk::SIZE as i32);

        // Hostile mobs are only spawned if they're underground or it's night time
        if spawn_chunk.y < 0 || clock.is_night() {
            continue 'outer;
        }

        let Some(chunk) = world_map.get_chunk(&spawn_chunk) else {
            continue;
        };

        let blocks = Blocks::get();
        let grass = blocks.get_id("grass");
        let stone = blocks.get_id("stone");
        let air = blocks.get_id("air");
        let surface_blocks = [grass, stone];
        let surface = Surface::new(chunk, &surface_blocks, air);

        let (group_size, mob_id) = random_mobs.choose_hostile(&mut rng);

        let mob_config = mobs.get_config(mob_id);

        for _ in 0..group_size {
            let x = rng.next_usize() % Chunk::SIZE;
            let z = rng.next_usize() % Chunk::SIZE;
            let mut spawn_position =
                BlockPosition::from(spawn_chunk) + BlockPosition::new(x as i32, 0, z as i32);

            let Some((y, _)) = surface[[x, z]] else {
                continue 'outer;
            };
            spawn_position.y += y as i32;

            let mut entity_commands = commands.spawn((
                Mob { id: mob_id },
                RandomMobType::Hostile,
                Transform::from_translation(spawn_position.as_dvec3() + DVec3::new(0.5, 1.0, 0.5)),
            ));

            (mob_config.spawn_function)(&mut entity_commands);

            mob_cap.hostile += 1;

            if mob_cap.at_hostile_capacity() {
                continue 'outer;
            }
        }
    }
}

fn despawn_mobs(
    mut commands: Commands,
    chunk_subscriptions: Res<ChunkSubscriptions>,
    mob_query: Query<(Entity, &GlobalTransform), With<Mob>>,
    mut player_query: Query<(&GlobalTransform, &mut MobCap), With<Player>>,
    despawned_mobs: Query<(Entity, &GlobalTransform, &RandomMobType), With<MobDespawn>>,
) {
    'outer: for (mob_entity, mob_transform) in mob_query.iter() {
        let chunk_position = ChunkPosition::from(mob_transform.translation());
        let Some(subscribers) = chunk_subscriptions.get_subscribers(&chunk_position) else {
            // If there are no subscribers, the chunk isn't loaded anymore, instantly despawn
            commands.entity(mob_entity).insert(MobDespawn);
            continue;
        };

        for player_entity in subscribers {
            let (player_transform, _) = player_query.get(*player_entity).unwrap();
            let distance = player_transform
                .translation()
                .distance_squared(mob_transform.translation());
            // TODO: Should this use the simulation distance? There's really no use in having
            // random mobs be simulated far away, and if fills up the mob cap so there won't be any
            // near players.
            let radius = (Chunk::SIZE as f64 * 4.0).powi(2);

            if distance < radius {
                continue 'outer;
            }
        }

        commands.entity(mob_entity).insert(MobDespawn);
    }

    for (entity, transform, mob_type) in despawned_mobs.iter() {
        let chunk_position = ChunkPosition::from(transform.translation());
        if let Some(subscribers) = chunk_subscriptions.get_subscribers(&chunk_position) {
            for subscriber in subscribers {
                let (_, mut mob_cap) = player_query.get_mut(*subscriber).unwrap();
                match mob_type {
                    RandomMobType::Hostile => {
                        mob_cap.hostile = mob_cap.hostile.saturating_sub(1);
                    }
                    RandomMobType::Friendly => {
                        mob_cap.friendly = mob_cap.friendly.saturating_sub(1);
                    }
                }
            }
        }
        commands.entity(entity).despawn();
    }
}

#[derive(Default)]
pub struct MobSoundCollection {
    random: Vec<String>,
    damage: Vec<String>,
    death: Vec<String>,
}

/// Plays a random sound from the mob's [MobSoundCollection] at random intervals
#[derive(Component)]
pub struct MobRandomSound {
    rng: Rng,
    timer: Timer,
}

impl Default for MobRandomSound {
    fn default() -> Self {
        let mut new = Self {
            rng: Rng::default(),
            timer: Timer::default(),
        };
        new.reset_timer();
        new
    }
}

impl MobRandomSound {
    fn reset_timer(&mut self) {
        self.timer = Timer::from_seconds(
            UniformDistribution::new(6.0, 9.0).sample(&mut self.rng),
            TimerMode::Once,
        );
    }
}

#[derive(Component)]
#[require(Transform)]
pub struct Mob {
    pub id: MobId,
}

#[derive(Component, Serialize, Deserialize, Clone)]
pub struct MobHealth {
    hearts: u32,
    max: u32,
    invincibility: Option<Timer>,
}

impl MobHealth {
    fn new(hearts: u32) -> Self {
        Self {
            hearts,
            max: hearts,
            invincibility: None,
        }
    }

    fn heal(&mut self, healing: u32) {
        self.hearts = self.hearts.saturating_add(healing).min(self.max);
    }

    fn damage(&mut self, damage: u32) {
        self.hearts = self.hearts.saturating_sub(damage);
    }

    fn is_dead(&self) -> bool {
        self.hearts == 0
    }

    fn is_invincible(&self) -> bool {
        self.invincibility.is_some()
    }

    fn tick_invincibility(&mut self, delta: Duration) -> bool {
        if let Some(timer) = &mut self.invincibility {
            timer.tick(delta);
            if timer.just_finished() {
                self.invincibility = None;
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    fn set_invincible(&mut self, time: f32) {
        self.invincibility = Some(Timer::from_seconds(time, TimerMode::Once));
    }
}

#[derive(Component)]
pub struct MobDespawn;

fn handle_hand_hits(
    items: Res<Items>,
    player_inventory_query: Query<(&Inventory, &Camera), With<Player>>,
    mut mob_hits: Query<(Entity, &Mob, &HandHits, &mut Physics, &MobHealth), Changed<HandHits>>,
    mut damage_events: MessageWriter<MobDamageEvent>,
) {
    for (mob_entity, mob, hits, mut physics, health) in mob_hits.iter_mut() {
        if health.is_invincible() {
            continue;
        }

        for player in hits.iter() {
            let (inventory, camera) = player_inventory_query.get(player).unwrap();
            let damage = if let Some(item) = inventory.held_item_stack().item() {
                let item_config = items.get_config(&item.id);
                if let Some(damage_json) = item_config.properties.get("damage") {
                    damage_json.as_u64().unwrap_or(1) as u32
                } else {
                    5
                }
            } else {
                5
            };

            let horizontal = camera.forward().xz().normalize() * 10.0;
            physics.velocity = DVec3::new(horizontal.x, 7.0, horizontal.y);

            damage_events.write(MobDamageEvent { mob_entity, damage });
        }
    }
}

#[derive(Message)]
struct MobDamageEvent {
    mob_entity: Entity,
    damage: u32,
}

const INVINCIBILITY_TIME: f64 = 0.5;

fn damage_mobs(
    mut commands: Commands,
    net: Res<Server>,
    time: Res<Time>,
    mobs: Res<Mobs>,
    mut mob_query: Query<(
        Entity,
        &mut Mob,
        &mut MobHealth,
        &mut Transform,
        Option<&mut ModelColor>,
    )>,
    mut damage_events: MessageReader<MobDamageEvent>,
    mut rng: Local<Rng>,
) {
    for (mob_entity, mob, mut health, mut mob_transform, mut maybe_color) in mob_query.iter_mut() {
        if !health.is_invincible() {
            continue;
        };

        let finished = health.tick_invincibility(time.delta());

        if health.is_dead()
            && let Some(timer) = &health.invincibility
        {
            let delta = (timer.elapsed_secs_f64() / 0.25).min(1.0);
            let mut r = mob_transform.rotation;
            r.z = 0.0;
            r.x = 0.0;
            r = r.normalize();
            mob_transform.rotation = r.slerp(
                r * DQuat::from_rotation_z(-std::f64::consts::FRAC_PI_2),
                delta,
            );
        }

        if finished {
            if health.is_dead() {
                // Despawn the mob after its invincibility frames end
                commands.entity(mob_entity).despawn();
            }

            if let Some(mut color) = maybe_color {
                *color = ModelColor::new(1.0, 1.0, 1.0, 1.0);
            }
        }
    }

    for damage_event in damage_events.read() {
        let Ok((mob_entity, mut mob, mut health, transform, mut maybe_color)) =
            mob_query.get_mut(damage_event.mob_entity)
        else {
            continue;
        };

        if health.is_invincible() {
            continue;
        }

        health.damage(damage_event.damage);

        if health.is_dead() {
            // Use the invincibility to keep the entity alive so a death animation can be shown.
            health.set_invincible(1.0);
        } else {
            health.set_invincible(INVINCIBILITY_TIME as f32);
        }

        let damage_red = ModelColor::new(1.0, 0.5, 0.5, 1.0);
        if let Some(color) = maybe_color.as_deref_mut() {
            *color = damage_red;
        } else {
            commands.entity(mob_entity).insert(damage_red);
        }

        let sounds = &mobs.get_config(mob.id).sounds;

        if health.is_dead() && !sounds.death.is_empty() {
            let sound_index = rng.next_usize() % sounds.death.len();
            net.broadcast(messages::Sound {
                position: Some(transform.translation),
                volume: 1.0,
                speed: 1.0,
                sound: sounds.death[sound_index].to_owned(),
            });
        } else if !sounds.damage.is_empty() {
            let sound_index = rng.next_usize() % sounds.damage.len();
            net.broadcast(messages::Sound {
                position: Some(transform.translation),
                volume: 1.0,
                speed: 1.0,
                sound: sounds.damage[sound_index].to_owned(),
            });
        }
    }
}

fn play_random_sound(
    net: Res<Server>,
    time: Res<Time>,
    mobs: Res<Mobs>,
    mut mob_query: Query<(
        &Mob,
        &GlobalTransform,
        &ModelVisibility,
        &mut MobRandomSound,
    )>,
) {
    for (mob, transform, visibility, mut random_sound) in mob_query.iter_mut() {
        if !visibility.is_visible() {
            continue;
        }

        let mob_config = mobs.get_config(mob.id);

        random_sound.timer.tick(time.delta());
        if random_sound.timer.just_finished() {
            random_sound.reset_timer();

            let sounds = &mob_config.sounds.random;

            if sounds.is_empty() {
                warn!(
                    "MobSoundPlayer is added to an entity, but the SoundHandle attached doesn't have any random sounds registered."
                );
                continue;
            }

            let sound_index = random_sound.rng.next_usize() % sounds.len();
            net.broadcast(messages::Sound {
                position: Some(transform.translation()),
                volume: 1.0,
                speed: 1.0,
                sound: sounds[sound_index].to_owned(),
            });
        }
    }
}

#[derive(Component, Default)]
struct MobHead {
    position: DVec3,
    target: Option<DVec3>,
    // Max rotation for the head
    max_yaw: f32,
    max_pitch: f32,
    follow_player: bool,
    // Goal rotation
    goal_yaw: f32,
    goal_pitch: f32,
    // Current head rotation
    yaw: f32,
    pitch: f32,
}

impl MobHead {
    pub fn new(head_position: DVec3, max_yaw: f32, max_pitch: f32) -> Self {
        Self {
            position: head_position,
            target: None,
            max_yaw,
            max_pitch,
            follow_player: false,
            goal_yaw: 0.0,
            goal_pitch: 0.0,
            yaw: 0.0,
            pitch: 0.0,
        }
    }

    pub fn look_at(&mut self, position: Option<DVec3>) {
        self.target = position;
    }
}

fn look_around(
    net: Res<Server>,
    time: Res<Time>,
    models: Res<Models>,
    chunk_subscriptions: Res<ChunkSubscriptions>,
    mut mob_query: Query<(Entity, &mut Transform, &mut MobHead, &Physics, &Model), With<Mob>>,
    mut rng: Local<Rng>,
) {
    for (entity, mut transform, mut head, physics, model) in mob_query.iter_mut() {
        // First we determine which way the head should be rotated. If the mob is standing still,
        // we also rotate the body.
        if let Some(target) = head.target {
            let head_position = transform.translation + head.position;
            let mut head_transform = Transform::from_translation(head_position);
            head_transform.look_at(target, DVec3::Y);
            let (yaw, pitch, _) = head_transform.rotation.to_euler(EulerRot::YXZ);
            head.goal_yaw = (yaw as f32).max(-head.max_yaw).min(head.max_yaw);
            head.goal_pitch = (pitch as f32).max(-head.max_pitch).min(head.max_pitch);
        } else if rng.next_f32() < 0.01 {
            if physics.velocity == DVec3::ZERO {
                let head_yaw = UniformDistribution::new(-FRAC_PI_2, FRAC_PI_2).sample(&mut rng);
                head.goal_yaw += head_yaw;
                head.goal_pitch = 0.0;
            } else {
                // While the mob is moving, rotate the head in random directions
                let (yaw, _, _) = transform.rotation.to_euler(EulerRot::YXZ);
                let head_yaw =
                    UniformDistribution::new(-head.max_yaw, head.max_yaw).sample(&mut rng);

                head.goal_yaw = yaw as f32 + head_yaw;
                head.goal_pitch = 0.0;
            }
        }

        // Determine the amount of rotation needed to arrive at the goal.
        let remaining_yaw = transform
            // We want the rotation relative to [1.0, 0.0]
            .right()
            .xz()
            .as_vec2()
            // Vec3 rotate clockwise because -Vec3::Z is forwards while Vec2 rotates counter
            // clockwise, so it needs to be inverted.
            .mul(Vec2::new(1.0, -1.0))
            .angle_to(Vec2::from_angle(head.goal_yaw - head.yaw));
        let remaining_pitch = head.goal_pitch - head.pitch;

        if remaining_yaw.abs() < f32::EPSILON && remaining_pitch.abs() < f32::EPSILON {
            continue;
        }

        // Limit the amount of rotation per tick
        let yaw = (time.delta_secs() * std::f32::consts::PI)
            .min(remaining_yaw.abs())
            .copysign(remaining_yaw);
        let pitch = (time.delta_secs() * std::f32::consts::PI)
            .min(remaining_pitch.abs())
            .copysign(remaining_pitch);

        if (head.yaw + yaw < head.max_yaw && head.yaw + yaw > -head.max_yaw)
            || head.pitch.abs() < head.goal_pitch.abs()
        {
            head.yaw = (head.yaw + yaw).clamp(-head.max_yaw, head.max_yaw);
            head.pitch = (head.pitch + pitch).clamp(-head.max_pitch, head.max_pitch);

            let Model::Asset(model_id) = model else {
                unreachable!()
            };

            let model_config = models.get_config(model_id);

            let Some(bone) = model_config.bones.get("head") else {
                warn!("Missing 'head' bone");
                continue;
            };

            let chunk_position = ChunkPosition::from(transform.translation);
            let Some(subscribers) = chunk_subscriptions.get_subscribers(&chunk_position) else {
                continue;
            };

            let rotation = Quat::from_rotation_y(head.yaw) * Quat::from_rotation_x(head.pitch);

            net.send_one(
                *subscribers.iter().take(1).next().unwrap(),
                messages::ModelUpdateTransform {
                    model_id: entity.index(),
                    bone: Some(*bone),
                    position: DVec3::ZERO,
                    rotation,
                    scale: Vec3::ONE,
                },
            );
        } else if physics.velocity == DVec3::ZERO {
            transform.rotation = transform.rotation * DQuat::from_rotation_y(yaw as f64);
        }
    }
}
