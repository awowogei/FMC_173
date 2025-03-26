use std::collections::HashSet;

use fmc::{
    bevy::math::DVec3,
    blocks::{BlockPosition, Blocks},
    database::Database,
    models::{AnimationPlayer, Model, ModelVisibility, Models},
    physics::{Collider, Physics},
    players::{Camera, Player},
    prelude::*,
    utils::Rng,
    world::{chunk::Chunk, WorldMap},
};

use crate::{
    players::{GameMode, HandHits, PlayerDamageEvent},
    settings::Settings,
    skybox::Clock,
};

use super::{
    pathfinding::PathFinder, Health, Mob, MobDespawnEvent, MobRandomSound, MobSounds,
    MobSpawnEvent, SoundCollection,
};

pub struct ZombiePlugin;
impl Plugin for ZombiePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup).add_systems(
            Update,
            (
                spawn_zombies,
                despawn_zombies.after(super::update_mob_map),
                wander,
                move_to_pathfinding_goal,
                hunt_player,
                hide_during_daytime,
                attack,
            ),
        );
    }
}

fn is_night_time(clock: &Res<Clock>) -> bool {
    clock.as_secs() > 700 && clock.as_secs() < 1100
}

#[derive(Component, Default)]
struct Zombie {
    wander_timer: Timer,
    // Player targeted
    target: Option<Entity>,
}

impl Zombie {
    const EYES: DVec3 = DVec3::new(0.0, 1.65, 0.0);
}

#[derive(Bundle)]
struct ZombieBundle {
    mob: Mob,
    zombie: Zombie,
    physics: Physics,
    path_finder: PathFinder,
    collider: Collider,
    hits: HandHits,
    random_sound: MobRandomSound,
}

impl Default for ZombieBundle {
    fn default() -> Self {
        Self {
            mob: Mob {
                health: Health::new(20),
                invincibility: None,
            },
            zombie: Zombie::default(),
            physics: Physics::default(),
            path_finder: PathFinder::new(1, 1),
            // TODO: This is done because aabbs are rotated during collision detection(blocks that are
            // rotatable use the same code). If it rotates when it is near a block it will phase because it
            // is wider in one direction. Unclear what to do about it. Just forcing it out when an
            // unsolvable collision happens will probably look weird as rotating would mean movement. Solve
            // for rotation collisions as well? hard.
            //
            // Make the zombie square even though its model is not. Also made a little bit smaller so it
            // will fit into gaps more easily
            collider: Collider::from_min_max(
                DVec3::new(-0.3, 0.0, -0.3),
                DVec3::new(0.3, 1.8, 0.3),
            ),
            hits: HandHits::default(),
            random_sound: MobRandomSound::default(),
        }
    }
}

fn setup(database: Res<Database>, mut mobs: ResMut<MobSounds>) {
    let connection = database.get_write_connection();
    connection
        .execute(
            "create table if not exists zombies (
            x REAL,
            y REAL,
            z REAL,
            data BLOB,
            PRIMARY KEY (x,y,z)
         )",
            [],
        )
        .expect("Could not create 'zombies' table");

    let sounds = SoundCollection {
        random: vec![
            "zombie_moan_1.ogg".to_owned(),
            "zombie_moan_2.ogg".to_owned(),
            "zombie_moan_3.ogg".to_owned(),
        ],
        damage: vec!["zombie_damage.ogg".to_owned()],
        death: vec!["zombie_death.ogg".to_owned()],
    };

    mobs.register("zombie", sounds)
}

fn spawn_zombies(
    mut commands: Commands,
    settings: Res<Settings>,
    clock: Res<Clock>,
    mob_sounds: Res<MobSounds>,
    // database: Res<Database>,
    // world_map: Res<WorldMap>,
    models: Res<Models>,
    mut spawn_events: EventReader<MobSpawnEvent>,
) {
    for spawn_event in spawn_events.read() {
        // x position is left 32 bits and z position the right 32 bits. z must be converted to u32
        // first because it will just fill the left 32 bits with junk. World seed is used to change
        // which chunks are next to each other.
        let seed = ((spawn_event.position.x as u64) << 32 | spawn_event.position.z as u32 as u64)
            .overflowing_mul(settings.seed)
            .0;
        let mut rng = Rng::new(seed);
        if rng.next_u32() % 10 != 0 {
            continue;
        }

        let group_size = rng.next_u32() % 3 + 1;

        for _ in 0..group_size {
            let x = rng.next_u32() as usize % Chunk::SIZE;
            let z = rng.next_u32() as usize % Chunk::SIZE;
            // This handles the entire column of chunks. If the surface isn't found here, the
            // rng is deterministic, and it will check again in the chunks above and below.
            let Some((y, block_id)) = spawn_event.surface[x << 4 | z] else {
                continue;
            };

            if block_id != Blocks::get().get_id("grass") {
                continue;
            }

            let spawn_position = BlockPosition::from(spawn_event.position)
                + BlockPosition::new(x as i32, y as i32, z as i32);

            let zombie_model = models.get_by_name("zombie");

            let mut animations = AnimationPlayer::default();
            animations.set_move_animation(Some(zombie_model.animations["wander"]));
            animations.set_idle_animation(Some(zombie_model.animations["idle"]));
            animations.set_transition_time(1.0);

            commands.spawn((
                ZombieBundle::default(),
                mob_sounds.get_handle("zombie"),
                Model::Asset(zombie_model.id),
                if is_night_time(&clock) {
                    ModelVisibility::Visible
                } else {
                    ModelVisibility::Hidden
                },
                animations,
                Transform::from_translation(spawn_position.as_dvec3() + DVec3::new(0.5, 1.0, 0.5)),
            ));
        }
    }
}

fn despawn_zombies(
    mut commands: Commands,
    zombies: Query<(), With<Zombie>>,
    mut despawn_events: EventReader<MobDespawnEvent>,
) {
    for despawn_event in despawn_events.read() {
        if zombies.get(despawn_event.entity).is_ok() {
            commands.entity(despawn_event.entity).despawn_recursive();
        }
    }
}

fn hunt_player(
    world_map: Res<WorldMap>,
    models: Res<Models>,
    players: Query<(Entity, &GameMode, &GlobalTransform, &Camera), With<Player>>,
    mut zombies: Query<(
        &mut Zombie,
        &mut PathFinder,
        &mut AnimationPlayer,
        &HandHits,
        &GlobalTransform,
        &ModelVisibility,
    )>,
) {
    for (
        mut zombie,
        mut path_finder,
        mut animation_player,
        hand_hits,
        zombie_transform,
        visibility,
    ) in zombies.iter_mut()
    {
        if !visibility.is_visible() {
            continue;
        }

        if let Some(player_entity) = hand_hits.iter().last() {
            zombie.target = Some(player_entity);
        } else if zombie.target.is_none() {
            for (player_entity, game_mode, player_transform, camera) in players.iter() {
                if *game_mode != GameMode::Survival
                    || zombie_transform
                        .translation()
                        .distance_squared(player_transform.translation())
                        > 100.0
                    || zombie_transform
                        .forward()
                        .dot(player_transform.translation() - zombie_transform.translation())
                        < 0.0
                {
                    continue;
                }

                let mut transform = Transform {
                    translation: zombie_transform.translation() + Zombie::EYES,
                    ..default()
                };
                transform.look_at(
                    player_transform.translation() + camera.translation,
                    DVec3::Y,
                );
                let mut raycast = world_map.raycast(&transform, 10.0);
                let mut hit = false;
                let blocks = Blocks::get();
                let player_block_position = BlockPosition::from(player_transform.translation());
                while let Some(block_id) = raycast.next_block() {
                    if blocks.get_config(&block_id).is_solid() {
                        hit = true;
                        break;
                    } else if raycast.position() == player_block_position {
                        break;
                    }
                }

                if hit {
                    continue;
                } else {
                    zombie.target = Some(player_entity);
                }
            }

            if zombie.target.is_none() {
                continue;
            }
        }

        let zombie_model = models.get_by_name("zombie");

        let Ok((_, game_mode, player_transform, _)) = players.get(zombie.target.unwrap()) else {
            // Player might disconnect
            zombie.target = None;
            animation_player.set_transition_time(1.0);
            animation_player.set_move_animation(Some(zombie_model.animations["wander"]));
            animation_player.set_idle_animation(Some(zombie_model.animations["idle"]));
            continue;
        };

        if zombie_transform
            .translation()
            .distance_squared(player_transform.translation())
            > 100.0
            || *game_mode != GameMode::Survival
        {
            // Lose interest
            zombie.target = None;
            animation_player.set_transition_time(1.0);
            animation_player.set_move_animation(Some(zombie_model.animations["wander"]));
            animation_player.set_idle_animation(Some(zombie_model.animations["idle"]));
            continue;
        }

        // noop on consecutive iterations where the target is set.
        // Move slowly into the hunt animation so it looks like the zombie slowly notices the player
        animation_player.set_transition_time(1.0);
        animation_player.set_move_animation(Some(zombie_model.animations["hunt"]));
        animation_player.set_idle_animation(Some(zombie_model.animations["hunt_idle"]));
        animation_player.set_transition_time(0.2);

        let mut offset = player_transform.translation() - zombie_transform.translation();
        offset.y = 0.0;
        offset = offset.normalize();

        path_finder.find_path(
            &world_map,
            zombie_transform.translation(),
            player_transform.translation() - offset,
        );
    }
}

fn wander(
    world_map: Res<WorldMap>,
    time: Res<Time>,
    mut zombies: Query<(
        &mut Zombie,
        &mut PathFinder,
        &GlobalTransform,
        &ModelVisibility,
    )>,
    mut rng: Local<Rng>,
) {
    for (mut zombie, mut path_finder, transform, visibility) in zombies.iter_mut() {
        if !visibility.is_visible() {
            continue;
        }

        zombie.wander_timer.tick(time.delta());

        if zombie.wander_timer.finished() {
            zombie.wander_timer = Timer::from_seconds(rng.range_f32(15.0..=30.0), TimerMode::Once);
        } else {
            continue;
        }

        let mut already_visited = HashSet::new();
        let mut potential_blocks = Vec::new();

        let blocks = Blocks::get();
        let water_id = blocks.get_id("surface_water");

        let start = BlockPosition::from(transform.translation());
        potential_blocks.push((start, u32::MIN, 0));
        already_visited.insert(start);

        let max_distance = rng.range_u32(1..=8);

        let mut index = 0;
        while let Some((block_position, mut score, mut distance)) =
            potential_blocks.get(index).cloned()
        {
            index += 1;

            distance += 1;
            if distance > max_distance {
                continue;
            }

            for offset in [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z] {
                let block_position = block_position + offset;

                if !already_visited.insert(block_position) {
                    continue;
                }

                // Always increase score, to always move as far as possible
                score += 1;

                let Some(block_id) = world_map.get_block(block_position) else {
                    continue;
                };
                let block_config = blocks.get_config(&block_id);

                if block_config.is_solid() {
                    // Try to jump one block up
                    let above = block_position + IVec3::Y;
                    let block_config = if let Some(block_id) = world_map.get_block(above) {
                        blocks.get_config(&block_id)
                    } else {
                        continue;
                    };
                    if !block_config.is_solid() {
                        potential_blocks.push((above, score, distance));
                    }
                } else if block_id == water_id {
                    // If in water, stay in the shallows
                    for step in 1..4i32 {
                        let below = block_position - IVec3::Y * step;
                        let block_config = if let Some(block_id) = world_map.get_block(below) {
                            blocks.get_config(&block_id)
                        } else {
                            break;
                        };
                        if block_config.is_solid() {
                            potential_blocks.push((block_position, score, distance));
                            break;
                        }
                    }
                    potential_blocks.push((block_position, score, distance));
                } else {
                    for step in 1..=2i32 {
                        let below = block_position - IVec3::Y * step;
                        let block_config = if let Some(block_id) = world_map.get_block(below) {
                            blocks.get_config(&block_id)
                        } else {
                            break;
                        };

                        if block_config.is_solid() {
                            potential_blocks.push((below + IVec3::Y, score, distance));
                            break;
                        } else {
                            // Prefer walking down, will hopefully lead to the shore (or a hole if
                            // unlucky)
                            score += 1;
                        }
                    }
                }
            }
        }

        let mut best_position = None;
        let mut max_score = 0;
        for (block_position, score, _distance) in potential_blocks {
            if score > max_score {
                best_position = Some(block_position);
                max_score = score;
            }
        }

        if let Some(best_position) = best_position {
            let goal = best_position.as_dvec3() + DVec3::new(0.5, 0.0, 0.5);
            path_finder.find_path(&world_map, transform.translation(), goal);
        }
    }
}

// Formula for how much speed you need to reach a height
// sqrt(2 * gravity * wanted height(1.4)) + some for air resistance
const JUMP_VELOCITY: f64 = 9.0;
const HUNTING_ACCELERATION: f64 = 30.0;
const WANDER_ACCELERATION: f64 = 10.0;

fn move_to_pathfinding_goal(
    time: Res<Time>,
    mut zombies: Query<
        (&Mob, &Zombie, &mut PathFinder, &mut Physics, &mut Transform),
        Or<(Changed<GlobalTransform>, Changed<PathFinder>)>,
    >,
) {
    for (mob, zombie, mut path_finder, mut physics, mut transform) in zombies.iter_mut() {
        // Mob entities are kept for a little while after death to show a death pose
        if mob.health.is_dead() {
            continue;
        }

        if let Some(next_position) = path_finder.next_shortcut(transform.translation) {
            let new_rotation = transform.looking_at(next_position, DVec3::Y).rotation;
            transform.rotation = transform
                .rotation
                .slerp(new_rotation, time.delta_secs_f64() / 0.20);
            let direction = transform.forward();

            // Only rotate around the Y-axis
            transform.rotation.x = 0.0;
            transform.rotation.z = 0.0;
            transform.rotation = transform.rotation.normalize();

            // TODO: Should not jump out of water, accelerate only so it looks more like a step up.
            if next_position.y - transform.translation.y > 0.1
                // Jump only when it hits a wall
                && (physics.grounded.x || physics.grounded.z)
                && physics.grounded.y
            {
                physics.velocity.y += JUMP_VELOCITY;
            }

            let mut acceleration = if zombie.target.is_some() {
                HUNTING_ACCELERATION
            } else {
                WANDER_ACCELERATION
            };

            if !physics.grounded.y {
                acceleration *= 0.1;
            }

            // TODO: Needs states for when grounded/swimming/falling and differing speeds.
            physics.acceleration.x += direction.x * acceleration;
            physics.acceleration.z += direction.z * acceleration;
        }
    }
}

// TODO: They can't just appeaar all at the same time
fn hide_during_daytime(
    mut zombies: Query<&mut ModelVisibility, With<Zombie>>,
    clock: Res<Clock>,
    mut hidden: Local<bool>,
) {
    let night_time = is_night_time(&clock);
    if !*hidden && !night_time {
        *hidden = true;
        for mut visibility in zombies.iter_mut() {
            *visibility = ModelVisibility::Hidden;
        }
    } else if *hidden && night_time {
        *hidden = false;
        for mut visibility in zombies.iter_mut() {
            *visibility = ModelVisibility::Visible;
        }
    }
}

fn attack(
    zombies: Query<(&Zombie, &GlobalTransform)>,
    players: Query<&GlobalTransform, With<Player>>,
    mut damage_event_writer: EventWriter<PlayerDamageEvent>,
) {
    for (zombie, zombie_transform) in zombies.iter() {
        let Some(target) = zombie.target else {
            continue;
        };
        let Ok(player_transform) = players.get(target) else {
            continue;
        };

        if zombie_transform
            .translation()
            .distance_squared(player_transform.translation())
            < 4.0
        {
            let horizontal = zombie_transform.forward().xz().normalize() * 15.0;
            let knock_back = DVec3::new(horizontal.x, 7.0, horizontal.y);
            damage_event_writer.send(PlayerDamageEvent {
                player_entity: target,
                damage: 5,
                knock_back: Some(knock_back),
            });
        }
    }
}
