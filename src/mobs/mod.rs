use std::collections::{HashMap, HashSet};

use fmc::{
    blocks::Blocks,
    prelude::*,
    world::{chunk::ChunkPosition, ChunkSimulationEvent, Surface, WorldMap},
};

mod duck;
mod pathfinding;
mod zombie;

pub struct MobsPlugin;
impl Plugin for MobsPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<MobSpawnEvent>()
            .add_event::<MobDespawnEvent>()
            .insert_resource(MobMap::default())
            .add_plugins(duck::DuckPlugin)
            .add_plugins(zombie::ZombiePlugin)
            .add_systems(Update, update_mob_map);
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
            despawn_event_writer.send(MobDespawnEvent { entity });
        }
    }

    for simulation_event in chunk_simulation_events.read() {
        match simulation_event {
            ChunkSimulationEvent::Start(chunk_position) => {
                mob_map
                    .position2entity
                    .insert(*chunk_position, HashSet::new());

                let Some(chunk) = world_map.get_chunk(chunk_position) else {
                    continue;
                };

                let surface = Surface::new(chunk, Blocks::get().get_id("air"));
                spawn_event_writer.send(MobSpawnEvent {
                    position: *chunk_position,
                    surface,
                });
            }
            ChunkSimulationEvent::Stop(chunk_position) => {
                let entities = mob_map.remove(chunk_position);
                despawn_event_writer.send_batch(
                    entities
                        .into_iter()
                        .map(|entity| MobDespawnEvent { entity }),
                );
            }
        }
    }
}

#[derive(Component, Default)]
pub struct Mob {
    despawn: bool,
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
    position2entity: HashMap<ChunkPosition, HashSet<Entity>>,
    entity2position: HashMap<Entity, ChunkPosition>,
}

impl MobMap {
    pub fn get_entities(&self, chunk_position: &ChunkPosition) -> Option<&HashSet<Entity>> {
        return self.position2entity.get(chunk_position);
    }

    fn insert_or_move(&mut self, chunk_position: ChunkPosition, entity: Entity) -> bool {
        if let Some(current_chunk_pos) = self.entity2position.get(&entity) {
            if current_chunk_pos == &chunk_position {
                return true;
            } else {
                self.position2entity
                    .get_mut(&current_chunk_pos)
                    .unwrap()
                    .remove(&entity);

                self.position2entity
                    .entry(chunk_position)
                    .or_insert(HashSet::new())
                    .insert(entity);

                self.entity2position.insert(entity, chunk_position);
            }
        } else if let Some(mobs) = self.position2entity.get_mut(&chunk_position) {
            mobs.insert(entity);
            self.entity2position.insert(entity, chunk_position);
        } else {
            return false;
        }

        return true;
    }

    fn remove(&mut self, chunk_position: &ChunkPosition) -> HashSet<Entity> {
        let entities = self.position2entity.remove(chunk_position).unwrap();
        for entity in entities.iter() {
            self.entity2position.remove(entity).unwrap();
        }
        entities
    }
}
