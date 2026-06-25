use std::collections::HashSet;

use fmc::{
    bevy::math::{DQuat, DVec3},
    blocks::{BlockPosition, Blocks},
    database::Database,
    items::{DropTable, Items},
    models::{AnimationPlayer, Model, ModelVisibility, Models},
    physics::{Collider, Physics},
    players::{Camera, Player},
    prelude::*,
    world::WorldMap,
};

use crate::{
    items::spawn_crates::MobCrates,
    players::{GameMode, HandHits, PlayerDamageEvent},
};

use super::{
    Mob, MobConfig, MobHealth, MobSoundCollection, Mobs, RandomMobs, Wanderer,
    pathfinding::PathFinder,
};

pub struct ZombiePlugin;
impl Plugin for ZombiePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(Update, (follow_path, hunt_player, attack));
    }
}

#[derive(Component, Default)]
struct Zombie {
    target: Option<Entity>,
}

impl Zombie {
    const EYES: DVec3 = DVec3::new(0.0, 1.65, 0.0);

    fn set_target(&mut self, target: Option<Entity>) {
        self.target = target;
    }
}

#[derive(Bundle)]
struct ZombieBundle {
    health: MobHealth,
    zombie: Zombie,
    physics: Physics,
    path_finder: PathFinder,
    collider: Collider,
    hits: HandHits,
    wanderer: Wanderer,
}

impl Default for ZombieBundle {
    fn default() -> Self {
        Self {
            health: MobHealth::new(20),
            zombie: Zombie::default(),
            physics: Physics::default(),
            path_finder: PathFinder::new(2, 1, 1),
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
            wanderer: Wanderer::new(0.0, 1.0),
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

    let zombie_model = models.get_config_by_name("zombie").unwrap();
    let zombie_id = zombie_model.id;

    let move_animation = zombie_model.animations["wander"];
    let idle_animation = zombie_model.animations["idle"];

    let spawn_zombie = move |commands: &mut EntityCommands| {
        let mut animation_player = AnimationPlayer::default();
        animation_player.set_move_animation(Some(move_animation));
        animation_player.set_idle_animation(Some(idle_animation));
        animation_player.set_transition_time(1.0);

        commands.insert((
            ZombieBundle::default(),
            Model::Asset(zombie_id),
            animation_player,
        ));
    };

    let sounds = MobSoundCollection {
        random: vec![
            "zombie_moan_1.ogg".to_owned(),
            "zombie_moan_2.ogg".to_owned(),
            "zombie_moan_3.ogg".to_owned(),
        ],
        damage: vec!["zombie_damage.ogg".to_owned()],
        death: vec!["zombie_death.ogg".to_owned()],
    };

    let feather = items.get_id("feather").unwrap();
    let mob_id = mobs.add_mob(MobConfig {
        spawn_function: Box::new(spawn_zombie),
        sounds,
        drop_table: DropTable::new(1.0, &[(feather, 1.0, 0, 2)]).unwrap(),
    });

    random_mobs.add_hostile(4, mob_id);

    let zombie_crate_id = items.get_id("zombie_crate").unwrap();
    mob_crates.add_crate(zombie_crate_id, mob_id);
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
            // target the player that last hit it
            zombie.set_target(Some(player_entity));
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
                    zombie.set_target(Some(player_entity));
                }
            }

            if zombie.target.is_none() {
                continue;
            }
        }

        let zombie_model = models.get_config_by_name("zombie").unwrap();

        let Ok((_, game_mode, player_transform, _)) = players.get(zombie.target.unwrap()) else {
            // Player might disconnect
            zombie.set_target(None);
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
            zombie.set_target(None);
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

        path_finder.find_path(
            &world_map,
            zombie_transform.translation(),
            player_transform.translation(),
        );
    }
}

// Formula for how much speed you need to reach a height
// sqrt(2 * gravity * wanted height(1.4)) + some for air resistance
const JUMP_VELOCITY: f64 = 9.0;
const HUNTING_ACCELERATION: f64 = 30.0;
const WANDER_ACCELERATION: f64 = 10.0;

fn follow_path(
    time: Res<Time>,
    mut zombies: Query<
        (
            &Mob,
            &MobHealth,
            &mut Zombie,
            &mut PathFinder,
            &mut Physics,
            &mut Transform,
        ),
        Or<(Changed<GlobalTransform>, Changed<PathFinder>)>,
    >,
) {
    for (mob, health, mut zombie, mut path_finder, mut physics, mut transform) in zombies.iter_mut()
    {
        // Mob entities are kept for a little while after death to show a death pose
        if health.is_dead() {
            continue;
        }

        let Some(next_position) = path_finder.next_node(transform.translation) else {
            continue;
        };

        let direction = (next_position - transform.translation)
            .with_y(0.0)
            .normalize();
        let rotation = DQuat::from_rotation_arc(DVec3::NEG_Z, direction);
        let max_rotation = time.delta_secs_f64() * std::f64::consts::TAU;
        transform.rotation = transform.rotation.rotate_towards(rotation, max_rotation);

        // TODO: Should not jump out of water, accelerate only so it looks more like a step up.
        if next_position.y - transform.translation.y > 0.1
            && physics.is_against_wall()
            && physics.is_grounded()
        {
            physics.velocity.y = JUMP_VELOCITY;
        }

        let mut acceleration = if zombie.target.is_some() {
            HUNTING_ACCELERATION
        } else {
            WANDER_ACCELERATION
        };

        if !physics.is_grounded() {
            acceleration *= 0.1;
        }

        // TODO: Needs states for when grounded/swimming/falling and differing speeds.
        physics.acceleration.x += direction.x * acceleration;
        physics.acceleration.z += direction.z * acceleration;
    }
}

fn attack(
    zombies: Query<(&Zombie, &GlobalTransform)>,
    players: Query<&GlobalTransform, With<Player>>,
    mut damage_event_writer: MessageWriter<PlayerDamageEvent>,
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
            damage_event_writer.write(PlayerDamageEvent {
                player_entity: target,
                damage: 5,
                knock_back: Some(knock_back),
            });
        }
    }
}
