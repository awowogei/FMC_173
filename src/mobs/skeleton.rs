use std::collections::HashSet;

use fmc::{
    bevy::math::{DQuat, DVec3},
    blocks::{BlockPosition, Blocks},
    items::{DropTable, Items},
    models::{AnimationPlayer, BoneAttachment, Model, ModelVisibility, Models},
    physics::{Collider, Physics},
    players::{Camera, Player},
    prelude::*,
    random::{Rng, UniformDistribution},
    world::WorldMap,
};

use crate::{
    items::{arrows::Arrow, spawn_crates::MobCrates},
    players::{GameMode, HandHits, PlayerDamageEvent},
    skybox::Clock,
};

use super::{
    Mob, MobConfig, MobHead, MobHealth, MobSoundCollection, Mobs, RandomMobs, Target, Wanderer,
    pathfinding::PathFinder,
};

pub struct SkeletonPlugin;
impl Plugin for SkeletonPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(Update, (follow_path, attack, add_bow));
    }
}

#[derive(Component)]
struct Skeleton {
    shot_timer: Timer,
    // Player targeted
    target: Option<Entity>,
    can_see_target: bool,
}

impl Skeleton {
    const EYES: DVec3 = DVec3::new(0.0, 1.65, 0.0);

    fn new() -> Self {
        Self {
            shot_timer: Timer::from_seconds(2.0, TimerMode::Once),
            target: None,
            can_see_target: false,
        }
    }

    fn set_target(&mut self, target: Option<Entity>) {
        self.can_see_target = target.is_some();
        self.target = target;
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
            wanderer: Wanderer::new(0.0, 1.0),
            target: Target::default(),
            mob_head: MobHead::new(
                Skeleton::EYES,
                std::f32::consts::FRAC_PI_8,
                std::f32::consts::FRAC_PI_8,
            ),
        }
    }
}

fn add_bow(mut commands: Commands, models: Res<Models>, skeletons: Query<Entity, Added<Skeleton>>) {
    for entity in skeletons.iter() {
        let skeleton_model = models.get_config_by_name("skeleton").unwrap();
        let skeleton_id = skeleton_model.id;
        let left_arm_bone_id = *skeleton_model.bones.get("left_arm").unwrap();

        let bow_model = models.get_config_by_name("bow").unwrap();
        let bow_id = bow_model.id;

        commands.entity(entity).with_children(|parent| {
            parent.spawn((
                Model::Asset(bow_id),
                BoneAttachment {
                    bone_id: left_arm_bone_id,
                },
                Transform {
                    translation: DVec3::new(0.0625, -0.5625, 0.0),
                    rotation: DQuat::from_euler(EulerRot::XYZ, 1.9, -1.55, -2.11),
                    scale: DVec3 {
                        x: 0.05,
                        y: 0.05,
                        z: 0.0625,
                    },
                },
            ));
        });
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
    let left_arm_bone_id = *skeleton_model.bones.get("left_arm").unwrap();

    let bow_model = models.get_config_by_name("bow").unwrap();
    let bow_id = bow_model.id;

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

    // let sounds = MobSoundCollection {
    //     random: vec![
    //         "zombie_moan_1.ogg".to_owned(),
    //         "zombie_moan_2.ogg".to_owned(),
    //         "zombie_moan_3.ogg".to_owned(),
    //     ],
    //     damage: vec!["zombie_damage.ogg".to_owned()],
    //     death: vec!["zombie_death.ogg".to_owned()],
    // };

    let feather = items.get_id("feather").unwrap();
    let mob_id = mobs.add_mob(MobConfig {
        spawn_function: Box::new(spawn_skeleton),
        sounds: MobSoundCollection::default(),
        drop_table: DropTable::new(1.0, &[(feather, 1.0, 0, 2)]).unwrap(),
    });

    random_mobs.add_hostile(4, mob_id);

    let skeleton_crate_id = items.get_id("skeleton_crate").unwrap();
    mob_crates.add_crate(skeleton_crate_id, mob_id);
}

fn attack(
    mut commands: Commands,
    time: Res<Time>,
    world_map: Res<WorldMap>,
    models: Res<Models>,
    player_query: Query<(&Transform, &Camera), With<Player>>,
    mut skeletons: Query<(
        &mut Skeleton,
        &mut PathFinder,
        &HandHits,
        &Transform,
        &mut Target,
    )>,
) {
    for (mut skeleton, mut path_finder, hand_hits, skeleton_transform, mut target) in
        skeletons.iter_mut()
    {
        if let Some(player_entity) = hand_hits.iter().last() {
            target.set(Some(player_entity));
        }

        let Some(player_entity) = target.get() else {
            continue;
        };

        let Ok((player_transform, camera)) = player_query.get(player_entity) else {
            continue;
        };

        if target.in_line_of_sight {
            skeleton.shot_timer.tick(time.delta());

            if skeleton.shot_timer.is_finished() {
                skeleton.shot_timer.reset();
            } else {
                continue;
            }

            let model_config = models.get_config_by_name("arrow").unwrap();

            let player_head = player_transform.translation + camera.translation;
            let skeleton_head = skeleton_transform.translation + Skeleton::EYES;
            let velocity = (player_head - skeleton_head).normalize() * 40.0;
            commands.spawn((
                Model::Asset(model_config.id),
                Arrow::new(velocity),
                Transform {
                    translation: skeleton_head,
                    rotation: DQuat::from_rotation_arc(DVec3::NEG_Z, velocity.normalize()),
                    scale: DVec3::new(0.0625, 0.0625, 0.0625),
                },
            ));
        } else {
            path_finder.find_path(
                &world_map,
                skeleton_transform.translation,
                player_transform.translation,
            );
        }
    }
}

// Formula for how much speed you need to reach a height
// sqrt(2 * gravity * wanted height(1.4)) + some for air resistance
const JUMP_VELOCITY: f64 = 9.0;

fn follow_path(
    time: Res<Time>,
    mut skeletons: Query<(
        &MobHealth,
        &Target,
        &mut Skeleton,
        &mut PathFinder,
        &mut Physics,
        &mut Transform,
    )>,
) {
    for (health, target, mut skeleton, mut path_finder, mut physics, mut transform) in
        skeletons.iter_mut()
    {
        // Death check because mob entities are kept for a little while after death to show a death pose.
        // Don't move while in line of sight, stand still and shoot
        if health.is_dead() || target.in_line_of_sight {
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

        // Skeletons stand still while shooting, but should still rotate towards the target
        if skeleton.can_see_target {
            continue;
        }

        // TODO: Should not jump out of water, accelerate only so it looks more like a step up.
        if next_position.y - transform.translation.y > 0.1
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
