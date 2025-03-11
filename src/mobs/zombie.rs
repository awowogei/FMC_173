use std::collections::HashSet;

use fmc::{
    bevy::math::DVec3,
    blocks::{BlockPosition, Blocks},
    models::{AnimationPlayer, Model, Models},
    physics::{Buoyancy, Collider, Physics},
    players::Player,
    prelude::*,
    world::{chunk::ChunkPosition, WorldMap},
};
use rand::Rng;

use crate::players::GameMode;

use super::pathfinding::PathFinder;

pub struct ZombiePlugin;
impl Plugin for ZombiePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                spawn_zombie,
                remove_zombie,
                wander,
                move_to_pathfinding_goal,
                hunt_player,
            ),
        );
    }
}

#[derive(Component, Default)]
struct Zombie {
    _focus: Option<DVec3>,
    wander_timer: Timer,
    // Player targeted
    target: Option<Entity>,
}

fn spawn_zombie(
    mut commands: Commands,
    world_map: Res<WorldMap>,
    models: Res<Models>,
    time: Res<Time>,
    zombie: Query<Entity, With<Zombie>>,
) {
    if time.elapsed_secs() < 1.0 || zombie.iter().count() == 1 {
        return;
    }
    if !world_map.contains_chunk(&ChunkPosition::new(64, 0, 16)) {
        return;
    }
    let zombie_model = models.get_by_name("zombie");

    let mut animations = AnimationPlayer::default();
    animations.set_move_animation(Some(zombie_model.animations["wander"]));
    animations.set_idle_animation(Some(zombie_model.animations["idle"]));
    animations.set_transition_time(1.0);

    // TODO: This is done because aabbs are rotated during collision detection(blocks that are
    // rotatable use the same code). If it rotates when it is near a block it will phase because it
    // is wider in one direction. Unclear what to do about it. Just forcing it out when an
    // unsolvable collision happens will probably look weird as rotating would mean movement. Solve
    // for rotation collisions as well? hard.
    //
    // Make the zombie square even though its model is not. Also made a little bit smaller so it
    // will fit into gaps more easily
    let collider = Collider::from_min_max(DVec3::new(-0.3, 0.0, -0.3), DVec3::new(0.3, 2.0, 0.3));

    commands.spawn((
        Zombie::default(),
        Model::Asset(zombie_model.id),
        animations,
        Transform::from_xyz(67.0, 7.0, 24.0),
        collider,
        Physics {
            buoyancy: Some(Buoyancy {
                density: 0.3,
                waterline: 0.4,
            }),
            ..default()
        },
        PathFinder::new(1, 1),
    ));
}

fn remove_zombie(
    mut commands: Commands,
    zombie: Query<Entity, With<Zombie>>,
    mut player: RemovedComponents<Player>,
) {
    for _removed in player.read() {
        commands.entity(zombie.single()).despawn_recursive();
    }
}

fn hunt_player(
    world_map: Res<WorldMap>,
    models: Res<Models>,
    players: Query<(Entity, &GameMode, &GlobalTransform), With<Player>>,
    mut zombies: Query<(
        &mut Zombie,
        &mut PathFinder,
        &mut AnimationPlayer,
        &GlobalTransform,
    )>,
) {
    for (mut zombie, mut path_finder, mut animation_player, zombie_transform) in zombies.iter_mut()
    {
        if zombie.target.is_none() {
            for (player_entity, game_mode, player_transform) in players.iter() {
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
                    translation: zombie_transform.translation() + DVec3::Y * 1.8,
                    ..default()
                };
                transform.look_at(player_transform.translation(), DVec3::Y);
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
                    let zombie_model = models.get_by_name("zombie");
                    // Move slowly into the hunt animation so it looks like the zombie slowly notices the player
                    animation_player.set_transition_time(1.0);
                    animation_player.set_move_animation(Some(zombie_model.animations["hunt"]));
                    animation_player.set_idle_animation(Some(zombie_model.animations["hunt_idle"]));
                    animation_player.set_transition_time(0.2);
                }
            }

            if zombie.target.is_none() {
                continue;
            }
        }

        let Ok((_, game_mode, player_transform)) = players.get(zombie.target.unwrap()) else {
            // Player might disconnect
            zombie.target = None;
            let zombie_model = models.get_by_name("zombie");
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
            let zombie_model = models.get_by_name("zombie");
            animation_player.set_transition_time(1.0);
            animation_player.set_move_animation(Some(zombie_model.animations["wander"]));
            animation_player.set_idle_animation(Some(zombie_model.animations["idle"]));
            continue;
        }

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
    mut zombies: Query<(&mut Zombie, &mut PathFinder, &GlobalTransform)>,
) {
    for (mut zombie, mut path_finder, transform) in zombies.iter_mut() {
        zombie.wander_timer.tick(time.delta());

        if zombie.wander_timer.finished() {
            zombie.wander_timer =
                Timer::from_seconds(rand::thread_rng().gen_range(5.0..=15.0), TimerMode::Once);
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

        let max_distance = rand::thread_rng().gen_range(1..=8);

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
        (&Zombie, &mut PathFinder, &mut Physics, &mut Transform),
        (Or<(Changed<GlobalTransform>, Changed<PathFinder>)>,),
    >,
) {
    for (zombie, mut path_finder, mut physics, mut transform) in zombies.iter_mut() {
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

            let acceleration = if zombie.target.is_some() {
                HUNTING_ACCELERATION
            } else {
                WANDER_ACCELERATION
            };

            // TODO: Needs states for when grounded/swimming/falling and differing speeds.
            physics.acceleration.x += direction.x * acceleration;
            physics.acceleration.z += direction.z * acceleration;
        }
    }
}
