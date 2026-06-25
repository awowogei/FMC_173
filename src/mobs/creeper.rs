use fmc::{
    bevy::math::{DQuat, DVec3},
    blocks::{BlockPosition, Blocks},
    items::{DropTable, Items},
    models::{AnimationPlayer, Model, ModelColor, Models},
    networking::Server,
    physics::{Collider, Physics},
    players::Player,
    prelude::*,
    protocol::messages,
    world::{BlockUpdate, ChunkSubscriptions, WorldMap, chunk::ChunkPosition},
};

use crate::{explosions::ExplosionEvent, items::spawn_crates::MobCrates, players::HandHits};

use super::{
    Mob, MobConfig, MobHead, MobHealth, MobSoundCollection, Mobs, RandomMobs, Target, Wanderer,
    pathfinding::PathFinder,
};

pub struct CreeperPlugin;
impl Plugin for CreeperPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(Update, (follow_path, actions));
    }
}

#[derive(Component, Default)]
struct Creeper {
    fuse: f32,
}

impl Creeper {
    const EYES: DVec3 = DVec3::new(0.0, 1.65, 0.0);
}

#[derive(Bundle)]
struct CreeperBundle {
    health: MobHealth,
    zombie: Creeper,
    physics: Physics,
    path_finder: PathFinder,
    collider: Collider,
    hits: HandHits,
    target: Target,
    mob_head: MobHead,
}

impl Default for CreeperBundle {
    fn default() -> Self {
        Self {
            health: MobHealth::new(20),
            zombie: Creeper::default(),
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
            target: Target::default(),
            mob_head: MobHead::new(
                Creeper::EYES,
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
    let model = models.get_config_by_name("creeper").unwrap();
    let model_id = model.id;

    let move_animation = model.animations["walk"];
    let idle_animation = model.animations["idle"];

    let spawn_function = move |commands: &mut EntityCommands| {
        let mut animation_player = AnimationPlayer::default();
        animation_player.set_move_animation(Some(move_animation));
        animation_player.set_idle_animation(Some(idle_animation));
        animation_player.set_transition_time(1.0);

        commands.insert((
            CreeperBundle::default(),
            Model::Asset(model_id),
            animation_player,
        ));
    };

    let sounds = MobSoundCollection {
        damage: vec!["mobs/creeper/damage.ogg".to_owned()],
        death: vec!["mobs/creeper/death.ogg".to_owned()],
        ..default()
    };

    let feather = items.get_id("feather").unwrap();
    let mob_id = mobs.add_mob(MobConfig {
        spawn_function: Box::new(spawn_function),
        sounds: sounds,
        drop_table: DropTable::new(1.0, &[(feather, 1.0, 0, 2)]).unwrap(),
    });

    random_mobs.add_hostile(1, mob_id);

    let crate_id = items.get_id("creeper_crate").unwrap();
    mob_crates.add_crate(crate_id, mob_id);
}

fn actions(
    mut commands: Commands,
    time: Res<Time>,
    net: Res<Server>,
    world_map: Res<WorldMap>,
    models: Res<Models>,
    player_query: Query<&Transform, With<Player>>,
    mut creeper_query: Query<
        (
            Entity,
            &mut Creeper,
            &MobHealth,
            &mut PathFinder,
            &HandHits,
            &mut Transform,
            &mut Target,
            &mut ModelColor,
        ),
        Without<Player>,
    >,
    mut explosion_events: MessageWriter<ExplosionEvent>,
) {
    for (
        creeper_entity,
        mut creeper,
        health,
        mut path_finder,
        hand_hits,
        mut transform,
        mut target,
        mut color,
    ) in creeper_query.iter_mut()
    {
        if let Some(player_entity) = hand_hits.iter().last() {
            target.set(Some(player_entity));
        }

        let Some(player_entity) = target.get() else {
            continue;
        };

        let Ok(player_transform) = player_query.get(player_entity) else {
            continue;
        };

        if player_transform.translation.distance(transform.translation) < 3.0
            && target.in_line_of_sight
        {
            if creeper.fuse == 0.0 {
                net.broadcast(messages::Sound {
                    position: Some(transform.translation),
                    volume: 1.0,
                    speed: 1.0,
                    sound: "fuse.ogg".to_owned(),
                });
            }
            creeper.fuse += time.delta_secs();
        } else {
            creeper.fuse -= time.delta_secs();
            creeper.fuse = creeper.fuse.max(0.0);
        }

        if creeper.fuse >= 1.5 {
            explosion_events.write(ExplosionEvent {
                position: transform.translation,
                radius: 3,
            });
            commands.entity(creeper_entity).despawn();
        } else if creeper.fuse > 0.0 {
            // Flash while ignited. If it is invincible it means it has been damaged, and is
            // colored red. This should not overwrite the damage color.
            if !health.is_invincible() {
                if creeper.fuse.fract() % 0.5 > 0.25 {
                    color.set_if_neq(ModelColor::new(2.0, 2.0, 2.0, 1.0));
                } else {
                    color.set_if_neq(ModelColor::WHITE);
                }
            }

            transform.scale = DVec3::splat(1.0 + creeper.fuse as f64 * 0.1);
        }

        path_finder.find_path(
            &world_map,
            transform.translation,
            player_transform.translation,
        );
    }
}

// Formula for how much speed you need to reach a height
// sqrt(2 * gravity * wanted height(1.4)) + some for air resistance
const JUMP_VELOCITY: f64 = 9.0;

fn follow_path(
    time: Res<Time>,
    mut creepers: Query<(
        &MobHealth,
        &Target,
        &mut Creeper,
        &mut PathFinder,
        &mut Physics,
        &mut Transform,
    )>,
) {
    for (health, target, mut creeper, mut path_finder, mut physics, mut transform) in
        creepers.iter_mut()
    {
        // Death check because mob entities are kept for a little while after death to show a death pose.
        // Don't move while in line of sight, stand still and shoot
        if health.is_dead() {
            continue;
        }

        let Some(next_position) = path_finder.next_node(transform.translation) else {
            continue;
        };

        // TODO: It should just walk into the player and be bounced by the physics, but that isn't
        // implemented yet.
        //
        // Stop when close to the player.
        if target
            .last_position
            .xz()
            .distance_squared(transform.translation.xz())
            < 1.0
        {
            continue;
        }

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

        let mut acceleration = 20.0;

        if !physics.is_grounded() {
            acceleration *= 0.1;
        }

        // TODO: Needs states for when grounded/swimming/falling and differing speeds.
        physics.acceleration.x += direction.x * acceleration;
        physics.acceleration.z += direction.z * acceleration;
    }
}
