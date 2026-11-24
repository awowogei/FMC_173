use fmc::{
    items::drop_table::DropTable,
    math::{DQuat, DVec3},
    models::{AnimationPlayer, Model, Models},
    networking::Server,
    physics::{Collider, Physics},
    players::{Camera, Player},
    prelude::*,
    protocol::messages,
    world::WorldMap,
};

use crate::{
    items::{Items, arrows::Arrow, spawn_crates::MobCrates},
    players::HandHits,
    world::blocks::Blocks,
};

use super::{
    MobConfig, MobHead, MobHealth, MobSoundCollection, Mobs, RandomMobs, Target, Wanderer,
    pathfinding::PathFinder,
};

pub struct SkeletonPlugin;
impl Plugin for SkeletonPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(Update, (follow_path, attack));
    }
}

#[derive(Component)]
struct Skeleton {
    shot_timer: Timer,
    stopped: bool,
}

impl Skeleton {
    const EYES: DVec3 = DVec3::new(0.0, 1.65, 0.0);

    fn new() -> Self {
        Self {
            shot_timer: Timer::from_seconds(2.0, TimerMode::Repeating),
            stopped: false,
        }
    }
}

#[derive(Bundle)]
struct SkeletonBundle {
    health: MobHealth,
    skeleton: Skeleton,
    physics: Physics,
    path_finder: PathFinder,
    collider: Collider,
    hits: HandHits,
    wanderer: Wanderer,
    target: Target,
    mob_head: MobHead,
}

impl Default for SkeletonBundle {
    fn default() -> Self {
        Self {
            health: MobHealth::new(20),
            skeleton: Skeleton::new(),
            physics: Physics::default(),
            path_finder: PathFinder::new(2, 1, 1),
            collider: Collider::from_min_max(
                DVec3::new(-0.3, 0.0, -0.3),
                DVec3::new(0.3, 1.8, 0.3),
            ),
            hits: HandHits::default(),
            wanderer: Wanderer::new(2.0, 4.0),
            target: Target::default(),
            mob_head: MobHead::new(
                Skeleton::EYES,
                std::f32::consts::FRAC_PI_8,
                std::f32::consts::FRAC_PI_8,
            ),
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
    let skeleton_model = models.get_config_by_name("skeleton").unwrap();
    let skeleton_id = skeleton_model.id;

    let move_animation = skeleton_model.animations["walk"];
    let idle_animation = skeleton_model.animations["idle"];

    let spawn_skeleton = move |commands: &mut EntityCommands| {
        let mut animation_player = AnimationPlayer::default();
        animation_player.set_move_animation(Some(move_animation));
        animation_player.set_idle_animation(Some(idle_animation));
        animation_player.set_transition_time(0.15);

        commands.insert((
            SkeletonBundle::default(),
            Model::Asset(skeleton_id),
            animation_player,
        ));
    };

    let sounds = MobSoundCollection {
        random: vec![
            "mobs/skeleton/random_1.ogg".to_owned(),
            "mobs/skeleton/random_2.ogg".to_owned(),
        ],
        damage: vec![
            "mobs/skeleton/random_1.ogg".to_owned(),
            "mobs/skeleton/random_2.ogg".to_owned(),
        ],
        death: vec!["mobs/skeleton/death.ogg".to_owned()],
    };

    let bone = items.get_id("bone").unwrap();
    let arrow = items.get_id("arrow").unwrap();
    let mob_id = mobs.add_mob(MobConfig {
        spawn_function: Box::new(spawn_skeleton),
        sounds,
        drop_tables: vec![
            DropTable::new(1.0, &[(bone, 1.0, 0, 2)]).unwrap(),
            DropTable::new(1.0, &[(arrow, 1.0, 0, 2)]).unwrap(),
        ],
    });

    random_mobs.add_hostile(4, mob_id);

    let skeleton_crate_id = items.get_id("skeleton_crate").unwrap();
    mob_crates.add_crate(skeleton_crate_id, mob_id);
}

fn attack(
    mut commands: Commands,
    time: Res<Time>,
    net: Res<Server>,
    world_map: Res<WorldMap>,
    blocks: Res<Blocks>,
    models: Res<Models>,
    player_query: Query<(&Transform, &Camera), With<Player>>,
    mut skeletons: Query<(
        &mut Skeleton,
        &MobHealth,
        &mut PathFinder,
        &HandHits,
        &Transform,
        &mut Target,
    )>,
) {
    for (mut skeleton, health, mut path_finder, hand_hits, skeleton_transform, mut target) in
        skeletons.iter_mut()
    {
        if health.is_dead() {
            continue;
        }

        if let Some(player_entity) = hand_hits.iter().last() {
            target.set(Some(player_entity));
        }

        let Some(player_entity) = target.get() else {
            continue;
        };

        let Ok((player_transform, camera)) = player_query.get(player_entity) else {
            continue;
        };

        if target.in_line_of_sight && skeleton.shot_timer.tick(time.delta()).just_finished() {
            let model_config = models.get_config_by_name("arrow").unwrap();

            let player_head = player_transform.translation + camera.translation;
            let skeleton_head = skeleton_transform.translation + Skeleton::EYES;
            let offset = player_head - skeleton_head;
            let flight_time = offset.length() / Arrow::SPEED;
            let drop = 0.5 * Arrow::GRAVITY * flight_time * flight_time;
            let velocity = (offset + DVec3::Y * drop).normalize() * Arrow::SPEED;
            commands.spawn((
                Model::Asset(model_config.id),
                Arrow::new(velocity),
                Transform {
                    translation: skeleton_head,
                    rotation: DQuat::from_rotation_arc(DVec3::NEG_Z, velocity.normalize()),
                    scale: DVec3::new(0.0625, 0.0625, 0.0625),
                },
            ));

            net.broadcast(messages::Sound {
                position: Some(skeleton_head),
                volume: 1.0,
                speed: 1.0,
                sound: "bow.ogg".to_owned(),
            });
        }

        path_finder.find_path(
            &world_map,
            &blocks,
            skeleton_transform.translation,
            target.last_position,
        );
    }
}

// Formula for how much speed you need to reach a height
// sqrt(2 * gravity * wanted height(1.4)) + some for air resistance
const JUMP_VELOCITY: f64 = 9.0;

fn follow_path(
    time: Res<Time>,
    mut skeletons: Query<(
        &mut Skeleton,
        &MobHealth,
        &Target,
        &mut PathFinder,
        &mut Physics,
        &mut Transform,
    )>,
) {
    for (mut skeleton, health, target, mut path_finder, mut physics, mut transform) in
        skeletons.iter_mut()
    {
        // Death check because mob entities are kept for a little while after death to show a death pose.
        if health.is_dead() {
            continue;
        }

        // Stand still when close enough, start chasing when too far away
        let distance = transform
            .translation
            .xz()
            .distance(target.last_position.xz());
        skeleton.stopped =
            target.in_line_of_sight && (distance <= 7.0 || (skeleton.stopped && distance <= 9.0));

        let look_at = if skeleton.stopped {
            target.last_position
        } else if let Some(next_position) = path_finder.next_node(transform.translation) {
            next_position
        } else {
            continue;
        };

        let Some(direction) = (look_at - transform.translation)
            .with_y(0.0)
            .try_normalize()
        else {
            continue;
        };

        let rotation = DQuat::from_rotation_arc(DVec3::NEG_Z, direction);
        transform.rotation = transform
            .rotation
            .rotate_towards(rotation, time.delta_secs_f64() * std::f64::consts::TAU);

        if skeleton.stopped {
            continue;
        }

        // TODO: Should not jump out of water, accelerate only so it looks more like a step up.
        if look_at.y - transform.translation.y > 0.1
            && physics.is_against_wall()
            && physics.is_grounded()
        {
            physics.velocity.y = JUMP_VELOCITY;
        }

        let mut acceleration = 20.0;

        if !physics.is_grounded() {
            acceleration *= 0.1;
        }

        // TODO: Needs states for when grounded/swimming/falling and differing speeds.
        physics.acceleration.x += direction.x * acceleration;
        physics.acceleration.z += direction.z * acceleration;
    }
}
