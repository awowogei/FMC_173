use std::collections::HashSet;

use fmc::{
    bevy::math::DVec3,
    blocks::{BlockPosition, Blocks},
    database::Database,
    items::Items,
    models::{AnimationPlayer, Model, ModelVisibility, Models},
    physics::{Collider, Physics},
    players::{Camera, Player},
    prelude::*,
    random::{Rng, UniformDistribution},
    world::WorldMap,
};

use crate::{
    items::spawn_crates::MobCrates,
    players::{GameMode, HandHits, PlayerDamageEvent},
    skybox::Clock,
};

use super::{
    Mob, MobConfig, MobHealth, MobSoundCollection, Mobs, RandomMobs, pathfinding::PathFinder,
};

pub struct CowPlugin;
impl Plugin for CowPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(Update, (find_wander_location, move_to_pathfinding_goal));
    }
}

#[derive(Component)]
struct Cow {
    wander_timer: Timer,
    rng: Rng,
}

impl Cow {
    const EYES: DVec3 = DVec3::new(0.0, 1.65, 0.0);

    fn new() -> Self {
        let mut cow = Self {
            wander_timer: Timer::default(),
            rng: Rng::new(0),
        };

        cow.reset_wander_timer();
        cow
    }

    fn reset_wander_timer(&mut self) {
        self.wander_timer = Timer::from_seconds(
            UniformDistribution::new(2.0, 8.0).sample(&mut self.rng),
            TimerMode::Once,
        );
    }
}

#[derive(Bundle)]
struct CowBundle {
    health: MobHealth,
    cow: Cow,
    physics: Physics,
    path_finder: PathFinder,
    collider: Collider,
    hits: HandHits,
}

impl Default for CowBundle {
    fn default() -> Self {
        Self {
            health: MobHealth::new(20),
            cow: Cow::new(),
            physics: Physics::default(),
            path_finder: PathFinder::new(1, 1),
            // TODO: This is done because aabbs are rotated during collision detection(blocks that are
            // rotatable use the same code). If it rotates when it is near a block it will phase because it
            // is wider in one direction. Unclear what to do about it. Just forcing it out when an
            // unsolvable collision happens will probably look weird as rotating would mean movement. Solve
            // for rotation collisions as well? hard.
            //
            // Make the cow square even though its model is not. Also made a little bit smaller so it
            // will fit into gaps more easily
            collider: Collider::from_min_max(
                DVec3::new(-0.45, 0.0, -0.45),
                DVec3::new(0.45, 1.4, 0.45),
            ),
            hits: HandHits::default(),
        }
    }
}

fn setup(
    database: Res<Database>,
    items: Res<Items>,
    mut mobs: ResMut<Mobs>,
    mut random_mobs: ResMut<RandomMobs>,
    mut mob_crates: ResMut<MobCrates>,
    models: Res<Models>,
) {
    // let connection = database.get_write_connection();
    // connection
    //     .execute(
    //         "create table if not exists zombies (
    //         x REAL,
    //         y REAL,
    //         z REAL,
    //         data BLOB,
    //         PRIMARY KEY (x,y,z)
    //      )",
    //         [],
    //     )
    //     .expect("Could not create 'zombies' table");

    let model = models.get_config_by_name("cow").unwrap();
    let cow_id = model.id;

    let move_animation = model.animations["walk"];
    let idle_animation = model.animations["idle"];

    let spawn_function = move |commands: &mut EntityCommands| {
        let mut animation_player = AnimationPlayer::default();
        animation_player.set_move_animation(Some(move_animation));
        animation_player.set_idle_animation(Some(idle_animation));
        animation_player.set_transition_time(0.15);

        commands.insert((CowBundle::default(), Model::Asset(cow_id), animation_player));
    };

    let sounds = MobSoundCollection::default();

    let mob_id = mobs.add_mob(MobConfig {
        spawn_function: Box::new(spawn_function),
        sounds,
    });

    random_mobs.add_friendly(4, mob_id);

    let cow_crate_id = items.get_id("cow_crate").unwrap();
    mob_crates.add_crate(cow_crate_id, mob_id);
}

fn find_wander_location(
    world_map: Res<WorldMap>,
    time: Res<Time>,
    mut cows: Query<(
        &mut Cow,
        &mut PathFinder,
        &GlobalTransform,
        &ModelVisibility,
    )>,
    mut rng: Local<Rng>,
) {
    for (mut cow, mut path_finder, transform, visibility) in cows.iter_mut() {
        if !visibility.is_visible() {
            continue;
        }

        cow.wander_timer.tick(time.delta());
        if !cow.wander_timer.just_finished() {
            continue;
        }

        let mut already_visited = HashSet::new();
        let mut potential_blocks: Vec<(BlockPosition, u32, u32)> = Vec::new();

        let blocks = Blocks::get();
        let water_id = blocks.get_id("surface_water");

        let start = BlockPosition::from(transform.translation());
        potential_blocks.push((start, u32::MIN, 0));
        already_visited.insert(start);

        let max_distance = UniformDistribution::<u32>::new(1, 8).sample(&mut rng);

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
const WALKING_ACCELERATION: f64 = 30.0;

fn move_to_pathfinding_goal(
    time: Res<Time>,
    mut cows: Query<
        (
            &Mob,
            &MobHealth,
            &mut Cow,
            &mut PathFinder,
            &mut Physics,
            &mut Transform,
        ),
        Or<(Changed<GlobalTransform>, Changed<PathFinder>)>,
    >,
) {
    for (mob, health, mut cow, mut path_finder, mut physics, mut transform) in cows.iter_mut() {
        // Mob entities are kept for a little while after death to show a death pose
        if health.is_dead() {
            continue;
        }

        if let Some(next_position) = path_finder.next_node(transform.translation) {
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
                physics.velocity.y = JUMP_VELOCITY;
            }

            let mut acceleration = WALKING_ACCELERATION;

            if !physics.grounded.y {
                acceleration *= 0.1;
            }

            // TODO: Needs states for when grounded/swimming/falling and differing speeds.
            physics.acceleration.x += direction.x * acceleration;
            physics.acceleration.z += direction.z * acceleration;
        } else {
            cow.reset_wander_timer();
        }
    }
}
