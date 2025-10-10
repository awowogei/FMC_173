use std::collections::HashSet;

use fmc::{
    bevy::math::{DQuat, DVec3},
    blocks::{BlockPosition, Blocks},
    database::Database,
    items::Items,
    models::{AnimationPlayer, Model, ModelVisibility, Models},
    networking::Server,
    physics::{Collider, Physics},
    players::{Camera, Player},
    prelude::*,
    protocol::messages,
    random::{Rng, UniformDistribution},
    world::{
        WorldMap,
        chunk::{Chunk, ChunkPosition},
    },
};

use crate::{
    items::spawn_crates::MobCrates,
    players::{GameMode, HandHits},
};

use super::{
    Mob, MobConfig, MobHead, MobHealth, MobSoundCollection, Mobs, RandomMobs,
    pathfinding::PathFinder,
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
    const EYES: DVec3 = DVec3::new(0.0, 1.375, -0.75);

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
            UniformDistribution::new(2.0, 5.0).sample(&mut self.rng),
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

        commands.insert((
            CowBundle::default(),
            Model::Asset(cow_id),
            animation_player,
            super::MobHead::new(
                Cow::EYES,
                std::f32::consts::FRAC_PI_8,
                std::f32::consts::FRAC_PI_8,
            ),
        ));
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

        if cow.wander_timer.finished() && !path_finder.has_goal() {
            cow.reset_wander_timer();
        }

        cow.wander_timer.tick(time.delta());
        if !cow.wander_timer.just_finished() {
            continue;
        }

        let blocks = Blocks::get();
        let grass_id = blocks.get_id("grass");

        let wander_distance = UniformDistribution::new(-8i32, 8);

        let mut potential_blocks = Vec::with_capacity(10);
        for _ in 0..10 {
            let x = wander_distance.sample(&mut rng);
            let y = wander_distance.sample(&mut rng);
            let z = wander_distance.sample(&mut rng);
            let block_position =
                BlockPosition::from(transform.translation()) + BlockPosition::new(x, y, z);

            let chunk_position = ChunkPosition::from(block_position);
            let Some(chunk) = world_map.get_chunk(&chunk_position) else {
                continue;
            };

            // TODO: This is much the same as [fmc::world::Surface]. It is too expensive to
            // construct for each position, but maybe it should be precomputed and stored in the
            // chunk? There are many things that make use of it.
            let chunk_index_xz = block_position.as_chunk_index() & !0b1111;
            for y in (0..Chunk::SIZE).rev() {
                let chunk_index = chunk_index_xz | y;
                let block_id = chunk[chunk_index];

                if !blocks.get_config(&block_id).is_solid() {
                    continue;
                }

                let mut score = 0;
                if block_id == grass_id {
                    score += 1
                };

                // Stay out of caves
                if chunk_position.y + y as i32 > 0 {
                    score += 1;
                }

                let position =
                    BlockPosition::from(chunk_position) + BlockPosition::from(chunk_index + 1);
                potential_blocks.push((score, position));
                break;
            }
        }

        potential_blocks.sort_by_key(|(score, _)| *score);
        let Some((_, best_position)) = potential_blocks.last() else {
            return;
        };

        let goal = best_position.as_dvec3() + DVec3::new(0.5, 0.0, 0.5);
        path_finder.find_path(&world_map, transform.translation(), goal);
    }
}

// Formula for how much speed you need to reach a height
// sqrt(2 * gravity * wanted height(1.4)) + some for air resistance
const JUMP_VELOCITY: f64 = 9.0;
const WALKING_ACCELERATION: f64 = 30.0;

fn move_to_pathfinding_goal(
    time: Res<Time>,
    mut cows: Query<(
        &MobHealth,
        &mut Cow,
        &mut PathFinder,
        &mut Physics,
        &mut Transform,
        &mut MobHead,
    )>,
) {
    for (health, mut cow, mut path_finder, mut physics, mut transform, mut mob_head) in
        cows.iter_mut()
    {
        // Mob entities are kept for a little while after death to show a death pose
        if health.is_dead() {
            continue;
        }

        if let Some(next_position) = path_finder.next_node(transform.translation) {
            let direction = (next_position - transform.translation)
                .with_y(0.0)
                .normalize();
            let rotation = DQuat::from_rotation_arc(DVec3::NEG_Z, direction);
            let max_rotation = time.delta_secs_f64() * std::f64::consts::TAU;
            transform.rotation = transform.rotation.rotate_towards(rotation, max_rotation);

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

            // if let Some(goal) = path_finder.goal() {
            //     let mut goal = goal.as_dvec3();
            //     // Keep the head level
            //     goal.y = transform.translation.y + Cow::EYES.y;
            //     mob_head.look_at(Some(goal));
            // } else {
            //     mob_head.look_at(None);
            // }

            // TODO: Needs states for when grounded/swimming/falling and differing speeds.
            physics.acceleration += transform.forward() * acceleration;
        }
    }
}
