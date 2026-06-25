use fmc::{
    bevy::math::{DQuat, DVec3},
    items::{DropTable, Items},
    models::{AnimationPlayer, Model, Models},
    physics::{Collider, Physics, shapes::Aabb},
    players::Player,
    prelude::*,
    random::Rng,
    world::WorldMap,
};

use crate::{
    items::spawn_crates::MobCrates,
    players::{HandHits, PlayerDamageEvent},
};

use super::{
    Mob, MobConfig, MobHealth, MobSoundCollection, Mobs, RandomMobs, Target, Wanderer,
    pathfinding::PathFinder,
};

pub struct SpiderPlugin;
impl Plugin for SpiderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(Update, (follow_path, actions));
    }
}

#[derive(Component)]
struct Spider {
    attack_timer: Timer,
    pounce_timer: Timer,
}

impl Spider {
    const EYES: DVec3 = DVec3::new(0.0, 1.65, 0.0);
    const COLLIDER: Collider = Collider::Single(Aabb {
        // TODO: This is the correct one, but rotating the spider horizontally causes the
        // collider to be rotated into the terrain.
        // center: DVec3::new(0.0, 7.0 / 16.0, -2.5 / 16.0),
        center: DVec3::new(0.0, 7.0 / 16.0, 0.0),
        half_extents: DVec3::new(12.0 / 16.0, 7.0 / 16.0, 12.0 / 16.0),
    });

    fn new() -> Self {
        Self {
            attack_timer: Timer::from_seconds(1.0, TimerMode::Once),
            pounce_timer: Timer::from_seconds(1.2, TimerMode::Once),
        }
    }
}

#[derive(Bundle)]
struct SpiderBundle {
    health: MobHealth,
    spider: Spider,
    physics: Physics,
    path_finder: PathFinder,
    collider: Collider,
    hits: HandHits,
    target: Target,
}

impl Default for SpiderBundle {
    fn default() -> Self {
        Self {
            health: MobHealth::new(20),
            spider: Spider::new(),
            physics: Physics::default(),
            path_finder: PathFinder::new(1, 2, 1),
            collider: Spider::COLLIDER,
            hits: HandHits::default(),
            target: Target::default(),
        }
    }
}

fn setup(
    items: Res<Items>,
    mut mobs: ResMut<Mobs>,
    mut random_mobs: ResMut<RandomMobs>,
    mut mob_crates: ResMut<MobCrates>,
    models: Res<Models>,
) {
    let model = models.get_config_by_name("spider").unwrap();
    let model_id = model.id;

    let move_animation = model.animations["walk"];
    let idle_animation = model.animations["idle"];

    let spawn_function = move |commands: &mut EntityCommands| {
        let mut animation_player = AnimationPlayer::default();
        animation_player.set_move_animation(Some(move_animation));
        animation_player.set_idle_animation(Some(idle_animation));
        animation_player.set_transition_time(0.15);

        commands.insert((
            SpiderBundle::default(),
            Model::Asset(model_id),
            animation_player,
        ));
    };

    let sounds = MobSoundCollection {
        random: vec![
            "mobs/spider/random_1.ogg".to_owned(),
            "mobs/spider/random_2.ogg".to_owned(),
            "mobs/spider/random_3.ogg".to_owned(),
        ],
        damage: vec![
            "mobs/spider/random_1.ogg".to_owned(),
            "mobs/spider/random_2.ogg".to_owned(),
            "mobs/spider/random_3.ogg".to_owned(),
        ],
        death: vec!["mobs/spider/death.ogg".to_owned()],
    };

    let feather = items.get_id("feather").unwrap();
    let mob_id = mobs.add_mob(MobConfig {
        spawn_function: Box::new(spawn_function),
        sounds,
        drop_table: DropTable::new(1.0, &[(feather, 1.0, 0, 2)]).unwrap(),
    });

    random_mobs.add_hostile(1, mob_id);

    let crate_id = items.get_id("spider_crate").unwrap();
    mob_crates.add_crate(crate_id, mob_id);
}

fn actions(
    mut commands: Commands,
    time: Res<Time>,
    world_map: Res<WorldMap>,
    models: Res<Models>,
    player_query: Query<&Transform, With<Player>>,
    mut spider_query: Query<
        (
            Entity,
            &MobHealth,
            &mut Spider,
            &mut PathFinder,
            &HandHits,
            &mut Transform,
            &mut Target,
            &mut Physics,
        ),
        Without<Player>,
    >,
    mut damage_event_writer: MessageWriter<PlayerDamageEvent>,
    mut rng: Local<Rng>,
) {
    for (
        entity,
        health,
        mut spider,
        mut path_finder,
        hand_hits,
        mut transform,
        mut target,
        mut physics,
    ) in spider_query.iter_mut()
    {
        if health.is_dead() {
            continue;
        }

        spider.attack_timer.tick(time.delta());
        spider.pounce_timer.tick(time.delta());

        if let Some(player_entity) = hand_hits.iter().last() {
            target.set(Some(player_entity));
        }

        let Some(player_entity) = target.get() else {
            continue;
        };

        let Ok(player_transform) = player_query.get(player_entity) else {
            continue;
        };

        let distance = player_transform.translation.distance(transform.translation);
        if distance < 1.5 && spider.attack_timer.is_finished() {
            spider.attack_timer.reset();

            let horizontal = transform.forward().xz().normalize() * 10.0;
            let knock_back = DVec3::new(horizontal.x, 7.5, horizontal.y);
            damage_event_writer.write(PlayerDamageEvent {
                player_entity,
                damage: 5,
                knock_back: Some(knock_back),
            });

            // TODO: There's no entity collision yet, so it has to be manually pushed back when it
            // does a pounce attack.
            if !physics.is_grounded() {
                physics.velocity.x *= -0.3;
                physics.velocity.z *= -0.3;
            }
        } else {
            path_finder.find_path(
                &world_map,
                transform.translation,
                player_transform.translation,
            );
        }

        if physics.is_grounded()
            && distance > 1.0
            && distance < 2.5
            && spider.pounce_timer.is_finished()
            && spider.attack_timer.is_finished()
        {
            spider.pounce_timer.reset();

            // Makes the pounce irregular
            if rng.next_u32() % 3 != 0 {
                continue;
            }

            let horizontal = transform.forward().xz() * 10.0;
            physics.velocity.x = horizontal.x;
            physics.velocity.z = horizontal.y;
            physics.velocity.y += JUMP_VELOCITY;
        }
    }
}

// Formula for how much speed you need to reach a height
// sqrt(2 * gravity * wanted height(1.4)) + some for air resistance
const JUMP_VELOCITY: f64 = 9.0;

fn follow_path(
    time: Res<Time>,
    mut spiders: Query<(
        &MobHealth,
        &Target,
        &mut Spider,
        &mut PathFinder,
        &mut Physics,
        &mut Transform,
    )>,
) {
    for (health, target, mut spider, mut path_finder, mut physics, mut transform) in
        spiders.iter_mut()
    {
        // Death check because mob entities are kept for a little while after death to show a death pose.
        if health.is_dead() {
            continue;
        }

        let mut position = transform.translation;

        // Spiders are two-wide so it needs special traversal logic to not get stuck. We use the
        // direction it is traveling to derive where the backside of the spider is. When the
        // backside reaches the pathfinding node it moves on to the next. This ensures that
        // whenever it takes a step to the side it always brings the whole body over, avoiding
        // walking straight into trees and such.
        // if let Some(prev_position) = path_finder.previous_node() {
        //     // This doesn't consume the next node cause it is never close enough.
        //     let Some(next_position) = path_finder.next_node(prev_position) else {
        //         continue;
        //     };
        //     let direction = next_position - prev_position;
        //
        //     if physics.against_north_south() && !physics.against_east_west() && direction.x != 0.0 {
        //         position.x += Spider::COLLIDER.as_aabb().half_extents.x * -direction.x.signum();
        //     } else if physics.against_east_west()
        //         && !physics.against_north_south()
        //         && direction.z != 0.0
        //     {
        //         position.z += Spider::COLLIDER.as_aabb().half_extents.z * -direction.z.signum();
        //     }
        // }

        let Some(next_position) = path_finder.next_node(position) else {
            continue;
        };

        let direction = (next_position - position).with_y(0.0).normalize();
        let view_direction = (next_position - transform.translation)
            .with_y(0.0)
            .normalize();
        let rotation = DQuat::from_rotation_arc(DVec3::NEG_Z, direction);
        let max_rotation = time.delta_secs_f64() * std::f64::consts::TAU;
        transform.rotation = transform.rotation.rotate_towards(rotation, max_rotation);

        // If walking into a wall, climb it
        if (physics.against_east_west() && direction.dot(DVec3::X).abs() > 0.6)
            || (physics.against_north_south() && direction.dot(DVec3::Z).abs() > 0.6)
        {
            // if next_position.y - position.y > 0.01 {
            physics.velocity.y = 2.5;
            // }
        }

        let mut acceleration = 30.0;

        if !physics.is_grounded() {
            acceleration *= 0.1;
        }

        // TODO: Needs states for when grounded/swimming/falling and differing speeds.
        physics.acceleration.x += direction.x * acceleration;
        physics.acceleration.z += direction.z * acceleration;
    }
}
