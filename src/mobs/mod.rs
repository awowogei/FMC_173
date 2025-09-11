use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use fmc::{
    bevy::math::{DQuat, DVec3},
    blocks::Blocks,
    items::Items,
    models::{ModelColor, ModelVisibility},
    networking::Server,
    physics::Physics,
    players::{Camera, Player},
    prelude::*,
    protocol::messages,
    random::{Rng, UniformDistribution},
    world::{ChunkSimulationEvent, Surface, WorldMap, chunk::ChunkPosition},
};
use serde::{Deserialize, Serialize};

use crate::players::{HandHits, Inventory};

mod duck;
mod pathfinding;
mod zombie;

pub struct MobsPlugin;
impl Plugin for MobsPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<MobSpawnEvent>()
            .add_event::<MobDespawnEvent>()
            .add_event::<MobDamageEvent>()
            .insert_resource(MobMap::default())
            .insert_resource(MobSounds::default())
            .add_plugins(duck::DuckPlugin)
            .add_plugins(zombie::ZombiePlugin)
            .add_systems(
                Update,
                (
                    update_mob_map,
                    handle_hand_hits,
                    damage_mobs,
                    play_random_sound,
                ),
            );
    }
}

fn update_mob_map(
    world_map: Res<WorldMap>,
    mut mob_map: ResMut<MobMap>,
    mobs: Query<(Entity, &GlobalTransform), (With<Mob>, Changed<GlobalTransform>)>,
    mut chunk_simulation_events: EventReader<ChunkSimulationEvent>,
    mut spawn_event_writer: EventWriter<MobSpawnEvent>,
    mut despawn_event_writer: EventWriter<MobDespawnEvent>,
) {
    for (entity, transform) in mobs.iter() {
        let chunk_position = ChunkPosition::from(transform.translation());

        if !mob_map.insert_or_move(chunk_position, entity) {
            despawn_event_writer.write(MobDespawnEvent { entity });
        }
    }

    for simulation_event in chunk_simulation_events.read() {
        match simulation_event {
            ChunkSimulationEvent::Start(chunk_position) => {
                mob_map
                    .position2entities
                    .insert(*chunk_position, HashSet::new());

                let Some(chunk) = world_map.get_chunk(chunk_position) else {
                    continue;
                };

                let blocks = Blocks::get();
                let surface = Surface::new(chunk, &[blocks.get_id("grass")], blocks.get_id("air"));
                spawn_event_writer.write(MobSpawnEvent {
                    position: *chunk_position,
                    surface,
                });
            }
            ChunkSimulationEvent::Stop(chunk_position) => {
                let entities = mob_map.remove(chunk_position);
                despawn_event_writer.write_batch(
                    entities
                        .into_iter()
                        .map(|entity| MobDespawnEvent { entity }),
                );
            }
        }
    }
}

#[derive(Resource, Default)]
struct MobSounds {
    name2index: HashMap<String, SoundHandle>,
    sounds: Vec<SoundCollection>,
}

impl MobSounds {
    fn register(&mut self, name: &str, sounds: SoundCollection) {
        self.name2index.insert(
            name.to_owned(),
            SoundHandle {
                id: self.sounds.len(),
            },
        );
        self.sounds.push(sounds);
    }

    fn get_handle(&self, name: &str) -> SoundHandle {
        self.name2index[name]
    }
}

struct SoundCollection {
    random: Vec<String>,
    damage: Vec<String>,
    death: Vec<String>,
}

#[derive(Component, Clone, Copy)]
struct SoundHandle {
    id: usize,
}

#[derive(Component)]
struct MobRandomSound {
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
pub struct Mob {
    health: Health,
    invincibility: Option<Timer>,
}

impl Mob {
    fn is_dead(&self) -> bool {
        self.health.is_dead()
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct Health {
    hearts: u32,
    max: u32,
}

impl Health {
    fn new(hearts: u32) -> Self {
        Self {
            hearts,
            max: hearts,
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
}

#[derive(Event)]
pub struct MobSpawnEvent {
    position: ChunkPosition,
    surface: Surface,
}

#[derive(Event)]
pub struct MobDespawnEvent {
    entity: Entity,
}

#[derive(Resource, Default)]
pub struct MobMap {
    position2entities: HashMap<ChunkPosition, HashSet<Entity>>,
    entity2position: HashMap<Entity, ChunkPosition>,
}

impl MobMap {
    pub fn get_entities(&self, chunk_position: &ChunkPosition) -> Option<&HashSet<Entity>> {
        return self.position2entities.get(chunk_position);
    }

    fn insert_or_move(&mut self, chunk_position: ChunkPosition, entity: Entity) -> bool {
        if let Some(current_chunk_pos) = self.entity2position.get(&entity) {
            if current_chunk_pos == &chunk_position {
                return true;
            } else {
                self.position2entities
                    .get_mut(current_chunk_pos)
                    .unwrap()
                    .remove(&entity);

                if let Some(mobs) = self.position2entities.get_mut(&chunk_position) {
                    mobs.insert(entity);
                } else {
                    return false;
                }

                self.entity2position.insert(entity, chunk_position);
            }
        } else if let Some(mobs) = self.position2entities.get_mut(&chunk_position) {
            mobs.insert(entity);
            self.entity2position.insert(entity, chunk_position);
        } else {
            return false;
        }

        return true;
    }

    fn remove(&mut self, chunk_position: &ChunkPosition) -> HashSet<Entity> {
        let entities = self.position2entities.remove(chunk_position).unwrap();
        for entity in entities.iter() {
            self.entity2position.remove(entity).unwrap();
        }
        entities
    }
}

fn handle_hand_hits(
    items: Res<Items>,
    player_inventory_query: Query<(&Inventory, &Camera), With<Player>>,
    mut mob_hits: Query<(Entity, &Mob, &HandHits, &mut Physics), Changed<HandHits>>,
    mut damage_events: EventWriter<MobDamageEvent>,
) {
    for (mob_entity, mob, hits, mut physics) in mob_hits.iter_mut() {
        if mob.invincibility.is_some() {
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

#[derive(Event)]
struct MobDamageEvent {
    mob_entity: Entity,
    damage: u32,
}

const INVINCIBILITY_TIME: f64 = 0.5;

fn damage_mobs(
    mut commands: Commands,
    net: Res<Server>,
    time: Res<Time>,
    mob_sounds: Res<MobSounds>,
    mut mobs: Query<(
        Entity,
        &mut Mob,
        &mut Transform,
        Option<&mut ModelColor>,
        Option<&SoundHandle>,
    )>,
    mut damage_events: EventReader<MobDamageEvent>,
    mut rng: Local<Rng>,
) {
    for (mob_entity, mob, mut mob_transform, mut maybe_color, _) in mobs.iter_mut() {
        // Split borrowing
        let mob = mob.into_inner();

        let Some(timer) = &mut mob.invincibility else {
            continue;
        };

        if mob.health.is_dead() {
            // We want to show the mob for a little longer than the normal invincibility time when
            // it dies.
            timer.set_duration(Duration::from_secs_f32(1.0));
        }

        if timer.elapsed_secs() == 0.0 {
            let damage_red = ModelColor::new(1.0, 0.5, 0.5, 1.0);
            if let Some(color) = maybe_color.as_deref_mut() {
                *color = damage_red;
            } else {
                commands.entity(mob_entity).insert(damage_red);
            }
        }

        timer.tick(time.delta());

        if mob.health.is_dead() {
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

        if timer.just_finished() {
            if mob.is_dead() {
                // Despawn the mob after its invincibility frames end
                commands.entity(mob_entity).despawn();
            }

            if let Some(mut color) = maybe_color {
                *color = ModelColor::new(1.0, 1.0, 1.0, 1.0);
            }

            mob.invincibility = None;
        }
    }

    for damage_event in damage_events.read() {
        let Ok((_, mut mob, transform, _, maybe_sound)) = mobs.get_mut(damage_event.mob_entity)
        else {
            continue;
        };

        if mob.invincibility.is_some() {
            continue;
        }

        mob.health.damage(damage_event.damage);
        mob.invincibility = Some(Timer::from_seconds(
            INVINCIBILITY_TIME as f32,
            TimerMode::Once,
        ));

        if let Some(handle) = maybe_sound {
            let sounds = &mob_sounds.sounds[handle.id];
            if mob.is_dead() && !sounds.death.is_empty() {
                let sound_index = rng.next_u32() as usize % sounds.death.len();
                net.broadcast(messages::Sound {
                    position: Some(transform.translation),
                    volume: 1.0,
                    speed: 1.0,
                    sound: sounds.death[sound_index].to_owned(),
                });
            } else if !sounds.damage.is_empty() {
                let sound_index = rng.next_u32() as usize % sounds.damage.len();
                net.broadcast(messages::Sound {
                    position: Some(transform.translation),
                    volume: 1.0,
                    speed: 1.0,
                    sound: sounds.damage[sound_index].to_owned(),
                });
            }
        }
    }
}

fn play_random_sound(
    net: Res<Server>,
    time: Res<Time>,
    mob_sounds: Res<MobSounds>,
    mut mobs: Query<(
        &GlobalTransform,
        &ModelVisibility,
        &mut MobRandomSound,
        &SoundHandle,
    )>,
) {
    for (transform, visibility, mut random_sound, handle) in mobs.iter_mut() {
        if !visibility.is_visible() {
            continue;
        }

        random_sound.timer.tick(time.delta());
        if random_sound.timer.just_finished() {
            random_sound.reset_timer();

            let sounds = &mob_sounds.sounds[handle.id].random;

            if sounds.is_empty() {
                warn!(
                    "MobRandomSound added to entity, but the SoundHandle attached doesn't have any random sounds registered."
                );
                continue;
            }

            let sound_index = random_sound.rng.next_u32() as usize % sounds.len();
            net.broadcast(messages::Sound {
                position: Some(transform.translation()),
                volume: 1.0,
                speed: 1.0,
                sound: sounds[sound_index].to_owned(),
            });
        }
    }
}
