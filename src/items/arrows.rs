use fmc::{
    bevy::math::DVec3,
    blocks::{BlockPosition, Blocks},
    items::Items,
    models::{Model, ModelMap},
    physics::{Collider, Physics},
    players::Player,
    prelude::*,
    world::{ChangedBlockEvent, WorldMap, chunk::ChunkPosition},
};
use std::collections::{HashMap, HashSet};

use super::{ItemRegistry, ItemUses};
use crate::{
    mobs::Mob,
    players::{HealEvent, Inventory, PlayerDamageEvent},
};

pub struct ArrowPlugin;
impl Plugin for ArrowPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, arrows);
    }
}

#[derive(Component)]
pub struct Arrow {
    despawn_timer: Option<Timer>,
    stuck_position: Option<BlockPosition>,
    velocity: DVec3,
}

impl Arrow {
    pub fn new(velocity: DVec3) -> Self {
        Self {
            despawn_timer: None,
            stuck_position: None,
            velocity,
        }
    }

    fn start_despawn_timer(&mut self) {
        self.despawn_timer = Some(Timer::from_seconds(120.0, TimerMode::Once));
    }
}

#[derive(Default)]
struct StuckArrows {
    arrows: HashMap<BlockPosition, HashSet<Entity>>,
}

impl StuckArrows {
    fn insert(&mut self, block_position: BlockPosition, entity: Entity) {
        self.arrows
            .entry(block_position)
            .or_default()
            .insert(entity);
    }

    fn remove(&mut self, block_position: BlockPosition, entity: Entity) {
        let arrows = self.arrows.get_mut(&block_position).unwrap();
        assert!(arrows.remove(&entity));
        if arrows.is_empty() {
            self.arrows.remove(&block_position);
        }
    }
}

fn arrows(
    mut commands: Commands,
    time: Res<Time>,
    world_map: Res<WorldMap>,
    model_map: Res<ModelMap>,
    mut arrow_query: Query<(Entity, &mut Arrow, &mut Transform)>,
    model_query: Query<
        (Entity, &Transform, &Collider, Has<Player>, Has<Mob>),
        (Without<Arrow>, With<Model>),
    >,
    mut block_updates: MessageReader<ChangedBlockEvent>,
    mut stuck_arrows: Local<StuckArrows>,
    mut player_damage_events: MessageWriter<PlayerDamageEvent>,
) {
    for (arrow_entity, mut arrow, mut transform) in arrow_query.iter_mut() {
        if let Some(timer) = &mut arrow.despawn_timer {
            timer.tick(time.delta());

            if timer.just_finished() {
                commands.entity(arrow_entity).despawn();
                if let Some(stuck_position) = arrow.stuck_position.take() {
                    stuck_arrows.remove(stuck_position, arrow_entity);
                }
            }

            continue;
        }

        transform.look_to(arrow.velocity, DVec3::Y);

        let max_distance = (arrow.velocity * time.delta_secs_f64()).length();

        for chunk_position in ChunkPosition::from(transform.translation).neighbourhood() {
            for (model_entity, model_transform, collider, is_player, is_mob) in
                model_query.iter_many(model_map.iter_entities(&chunk_position))
            {
                let Some((distance, _)) = collider.ray_intersection(model_transform, &transform)
                else {
                    continue;
                };
                if distance > max_distance {
                    continue;
                }

                transform.translation += arrow.velocity.normalize() * (distance - 0.3);
                transform.translation = transform.translation - model_transform.translation;
                commands.entity(model_entity).add_child(arrow_entity);

                arrow.start_despawn_timer();
                arrow.velocity = DVec3::ZERO;

                if is_player {}
            }
        }

        let blocks = Blocks::get();
        let mut friction = DVec3::ZERO;
        let mut raycast = world_map.raycast(&transform, max_distance);
        while let Some(block_id) = raycast.next_block() {
            let block_config = blocks.get_config(&block_id);
            if let Some(drag) = block_config.drag() {
                friction = friction.max(drag);
                continue;
            }

            // Minus a little to make the arrowhead stick out of the surface
            transform.translation += arrow.velocity.normalize() * (raycast.distance() - 0.3);
            arrow.start_despawn_timer();
            arrow.stuck_position = Some(raycast.position());
            arrow.velocity = DVec3::ZERO;
            break;
        }

        transform.translation += arrow.velocity * time.delta_secs_f64();

        let mass = 10.0;
        arrow.velocity.y -= 14.0 * time.delta_secs_f64();
        arrow.velocity *= (-friction / mass * time.delta_secs_f64()).exp()
    }

    for block_update in block_updates.read() {
        if let Some(arrows) = stuck_arrows.arrows.remove(&block_update.position) {
            for entity in arrows {
                let (_, mut arrow, _) = arrow_query.get_mut(entity).unwrap();
                arrow.despawn_timer = None;
            }
        }
    }
}
