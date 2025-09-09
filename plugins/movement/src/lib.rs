use std::collections::HashSet;

use fmc_client_api::{
    self as fmc,
    math::{BVec3, DVec3, Mat3},
    prelude::*,
};
use serde::Deserialize;

// sqrt(2 * gravity * wanted height(1.4)) + some for air resistance
const JUMP_VELOCITY: f32 = 9.0;
const GRAVITY: Vec3 = Vec3::new(0.0, -32.0, 0.0);
// TODO: I think this should be a thing only if you hold space. If you are skilled you can press
// space again as soon as you land if you have released it in the meantime.
// TODO: It feels nice when you jump up a block, but when jumping down it does nothing, feels like
// bouncing. Maybe replace with a jump timer when you land so it's constant? I feel like it would
// be better if you could jump faster when jumping downwards, but not as much as now.
//
// This is needed so that whenever you land early you can't just instantly jump again.
// v_t = v_0 * at => (v_t - v_0) / a = t
const JUMP_TIME: f32 = JUMP_VELOCITY * 1.7 / -GRAVITY.y;

type ModelId = u32;

#[derive(Default)]
struct MovementPlugin {
    properties: PlayerProperties,
    pressed_keys: HashSet<fmc::Key>,
    // Caching delta time to reduce interface hits
    delta_time: f32,
    models: HashSet<ModelId>,
    model_configs: Vec<CollisionConfig>,
    block_configs: Vec<CollisionConfig>,
    initialized: bool,
}

impl MovementPlugin {}

#[derive(Default)]
struct PlayerProperties {
    game_mode: GameMode,
    acceleration: Vec3,
    velocity: Vec3,
    climbing: Option<Vec3>,
    is_swimming: bool,
    is_grounded: BVec3,
    is_flying: bool,
    last_spacebar: f32,
    last_jump: f32,
}

#[derive(Default, PartialEq)]
enum GameMode {
    #[default]
    Survival,
    Creative,
    Spectator,
}

impl fmc::Plugin for MovementPlugin {
    fn update(&mut self) {
        if !self.initialized {
            return;
        }
        self.delta_time = fmc::delta_time();
        self.properties.last_jump += self.delta_time;
        self.properties.last_spacebar += self.delta_time;
        self.update_keyboard_input();
        match self.properties.game_mode {
            GameMode::Spectator => {
                self.flight_controller();
                self.no_collision();
            }
            GameMode::Creative => {
                if self.properties.is_flying {
                    self.flight_controller();
                } else {
                    self.walking_controller();
                }
                self.collision();
            }
            GameMode::Survival => {
                self.walking_controller();
                self.collision();
            }
        }
    }

    fn handle_server_data(&mut self, data: Vec<u8>) {
        #[derive(Deserialize)]
        enum Packet {
            Setup {
                blocks: Vec<CollisionConfig>,
                models: Vec<CollisionConfig>,
            },
            /// Changes the player's velocity
            Velocity(Vec3),
            /// Notifies the plugin of which models it should collide with.
            Models(Vec<ModelId>),
            /// Changes the game mode.
            Mode(u32),
        }

        let Ok(packet) = bincode::deserialize::<Packet>(&data) else {
            fmc::log("'Movement' plugin received malformed data from the server");
            return;
        };

        match packet {
            Packet::Setup { blocks, models } => {
                self.block_configs = blocks;
                self.model_configs = models;
                self.initialized = true;
            }
            Packet::Velocity(velocity) => self.properties.velocity += velocity,
            Packet::Models(models) => {
                self.models.clear();
                self.models.extend(models);
            }
            Packet::Mode(mode) => match mode {
                0 => {
                    self.properties.game_mode = GameMode::Survival;
                    self.properties.is_flying = false;
                }
                1 => self.properties.game_mode = GameMode::Creative,
                2 => {
                    self.properties.game_mode = GameMode::Spectator;
                    self.properties.is_flying = true;
                }
                _ => (),
            },
        }
    }

    fn set_update_frequency(&mut self) -> Option<f32> {
        Some(1.0 / 60.0)
    }

    fn new() -> Self
    where
        Self: Sized,
    {
        Self::default()
    }
}

fmc::register_plugin!(MovementPlugin);

impl MovementPlugin {
    fn update_keyboard_input(&mut self) {
        for key_update in fmc::keyboard_input() {
            if key_update.released {
                if self.properties.game_mode == GameMode::Creative
                    && key_update.key == fmc::Key::Space
                {
                    if self.properties.last_spacebar < 0.25 {
                        self.properties.is_flying = !self.properties.is_flying;
                        self.properties.velocity = Vec3::ZERO;
                    }
                    self.properties.last_spacebar = 0.0;
                }
                self.pressed_keys.remove(&key_update.key);
            } else {
                self.pressed_keys.insert(key_update.key);
            }
        }
    }

    fn walking_controller(&mut self) {
        let camera_transform = fmc::get_camera_transform();
        let camera_forward = camera_transform.forward();
        let forward = Vec3::new(camera_forward.x, 0., camera_forward.z);
        let sideways = Vec3::new(-camera_forward.z, 0., camera_forward.x);

        let mut standing_on_ladder = false;
        let mut horizontal_acceleration = Vec3::ZERO;
        let mut vertical_acceleration = Vec3::ZERO;

        for key in self.pressed_keys.iter() {
            match *key {
                fmc::Key::KeyW => horizontal_acceleration += forward,
                fmc::Key::KeyS => horizontal_acceleration -= forward,
                fmc::Key::KeyA => horizontal_acceleration -= sideways,
                fmc::Key::KeyD => horizontal_acceleration += sideways,
                fmc::Key::Space => {
                    if self.properties.is_swimming {
                        vertical_acceleration.y = 20.0
                    } else if self.properties.is_grounded.y && self.properties.last_jump > JUMP_TIME
                    {
                        self.properties.last_jump = 0.0;
                        self.properties.velocity.y = JUMP_VELOCITY;
                    }
                }
                fmc::Key::Shift => {
                    if self.properties.is_swimming {
                        vertical_acceleration.y = -30.0
                    } else if self.properties.climbing.is_some() {
                        self.properties.velocity.y = 0.0;
                        vertical_acceleration.y = 0.0;
                        standing_on_ladder = true;
                    }
                }
                _ => (),
            }
        }

        if !self.properties.is_swimming && !standing_on_ladder {
            vertical_acceleration += GRAVITY;
        }

        if horizontal_acceleration != Vec3::ZERO {
            horizontal_acceleration = horizontal_acceleration.normalize();

            if let Some(climbing_direction) = self.properties.climbing
                && climbing_direction.dot(horizontal_acceleration) < 0.0
            {
                self.properties.velocity.y = 5.0;
                vertical_acceleration.y = 0.0;
            }
        }

        if self.properties.is_swimming {
            if vertical_acceleration.y == 0.0 {
                vertical_acceleration.y = -10.0;
            }
            horizontal_acceleration.x *= 40.0;
            horizontal_acceleration.z *= 40.0;
        } else if self.properties.is_grounded.y {
            horizontal_acceleration *= 100.0;
        } else if self.properties.velocity.x.abs() > 2.0
            || self.properties.velocity.z.abs() > 2.0
            || self.properties.velocity.y < -10.0
        {
            // Move fast in air if you're already in motion
            horizontal_acceleration *= 50.0;
        } else {
            // Move slow in air in jumping from a standstill
            horizontal_acceleration *= 20.0;
        }

        self.properties.acceleration = horizontal_acceleration + vertical_acceleration;
    }

    fn flight_controller(&mut self) {
        let camera_transform = fmc::get_camera_transform();
        let camera_forward = camera_transform.forward();
        let forward = Vec3::new(camera_forward.x, 0., camera_forward.z);
        let sideways = Vec3::new(-camera_forward.z, 0., camera_forward.x);

        self.properties.velocity.y = 0.0;

        let mut acceleration = Vec3::ZERO;

        for key in self.pressed_keys.iter() {
            match *key {
                fmc::Key::KeyW => acceleration += forward,
                fmc::Key::KeyS => acceleration -= forward,
                fmc::Key::KeyA => acceleration -= sideways,
                fmc::Key::KeyD => acceleration += sideways,
                fmc::Key::Space => {
                    self.properties.velocity.y = JUMP_VELOCITY * 2.0;
                }
                fmc::Key::Shift => {
                    self.properties.velocity.y = -JUMP_VELOCITY * 2.0;
                }
                _ => (),
            }
        }

        if acceleration != Vec3::ZERO {
            acceleration = acceleration.normalize() * 140.0;
        }

        if self.pressed_keys.contains(&fmc::Key::Control) {
            acceleration *= 10.0;
        }

        self.properties.acceleration = acceleration;
    }

    fn no_collision(&mut self) {
        let player_transform = fmc::get_player_transform();
        let delta_time = Vec3::splat(self.delta_time);

        self.properties.velocity += self.properties.acceleration * delta_time;
        let new_position = player_transform.translation + self.properties.velocity * delta_time;

        if player_transform.translation != new_position {
            fmc::set_player_transform(Transform {
                translation: new_position,
                rotation: DQuat::IDENTITY,
                scale: Vec3::ONE,
            });
        }

        let friction: f32 = 0.9;
        self.properties.velocity =
            self.properties.velocity * (1.0 - friction).powf(4.0).powf(self.delta_time);
    }

    // TODO: This tunnels if you move faster than maybe a few blocks a second
    fn collision(&mut self) {
        let player_transform = fmc::get_player_transform();
        let delta_time = Vec3::splat(self.delta_time);

        self.properties.climbing = None;

        if self.properties.velocity.x != 0.0 {
            self.properties.is_grounded.x = false;
        }
        if self.properties.velocity.y != 0.0 {
            self.properties.is_grounded.y = false;
        }
        if self.properties.velocity.z != 0.0 {
            self.properties.is_grounded.z = false;
        }

        self.properties.velocity += self.properties.acceleration * delta_time;

        let was_swimming = self.properties.is_swimming;
        self.properties.is_swimming = false;

        let mut new_position = player_transform.translation + self.properties.velocity * delta_time;
        let mut move_back = Vec3::ZERO;
        let mut friction = Vec3::ZERO;
        for velocity in [
            Vec3::new(0.0, self.properties.velocity.y, 0.0),
            Vec3::new(self.properties.velocity.x, 0.0, self.properties.velocity.z),
        ] {
            let pos_after_move = player_transform
                .with_translation(player_transform.translation + velocity * delta_time);

            let player_aabb =
                Aabb::from_min_max(Vec3::new(-0.3, 0.0, -0.3), Vec3::new(0.3, 1.8, 0.3));
            let player_collider = Collider::Single(player_aabb.clone());

            for block_pos in player_collider.iter_block_positions(&pos_after_move) {
                let block_id = match fmc::get_block(block_pos) {
                    Some(id) => id,
                    // Disconnect? Should always have your surroundings loaded.
                    None => return,
                };

                let block_config = &self.block_configs[block_id as usize];

                let rotation = if let Some(block_state) = fmc::get_block_state(block_pos) {
                    BlockState(block_state).rotation()
                } else {
                    DQuat::IDENTITY
                };

                let block_transform = Transform {
                    translation: block_pos.as_vec3() + 0.5,
                    rotation,
                    ..Transform::IDENTITY
                };

                let Some(overlap) = player_collider.intersection(
                    &pos_after_move,
                    &block_transform,
                    &block_config.collider,
                ) else {
                    continue;
                };

                if block_config.climbable {
                    self.properties.climbing = Some((rotation * DVec3::Z).as_vec3());
                }

                if let Some(drag) = block_config.drag() {
                    friction = friction.max(drag);
                    if drag.y >= 0.5 {
                        self.properties.is_swimming = true;
                    }
                    continue;
                }

                Self::resolve_conflict(
                    &mut self.properties,
                    &mut move_back,
                    &mut friction,
                    &block_config,
                    velocity,
                    overlap,
                    delta_time,
                );
            }

            for model_id in fmc::get_models(player_aabb.min(), player_aabb.max()) {
                if !self.models.contains(&model_id) {
                    continue;
                }
                let model_config = &self.model_configs[model_id as usize];

                let model_transform = fmc::get_model_transform(model_id);

                let Some(intersection) = player_collider.intersection(
                    &pos_after_move,
                    &model_transform,
                    &model_config.collider,
                ) else {
                    continue;
                };

                Self::resolve_conflict(
                    &mut self.properties,
                    &mut move_back,
                    &mut friction,
                    &model_config,
                    velocity,
                    intersection,
                    delta_time,
                );
            }
        }

        new_position += move_back;

        if player_transform.translation != new_position {
            fmc::set_player_transform(Transform {
                translation: new_position,
                rotation: DQuat::IDENTITY,
                scale: Vec3::ONE,
            });
        }

        // XXX: Pow(4) is just to scale it further towards zero when friction is high. The function
        // should be read as 'velocity *= friction^time'
        self.properties.velocity =
            self.properties.velocity * (1.0 - friction).powf(4.0).powf(self.delta_time);

        // Give a little boost when exiting water so that the bob stays constant.
        if was_swimming && !self.properties.is_swimming {
            self.properties.velocity.y += 1.5;
        }
    }

    #[inline]
    fn resolve_conflict(
        properties: &mut PlayerProperties,
        move_back: &mut Vec3,
        friction: &mut Vec3,
        config: &CollisionConfig,
        velocity: Vec3,
        overlap: Vec3,
        delta_time: Vec3,
    ) {
        let backwards_time = overlap / -velocity;
        let valid_axes = backwards_time.cmplt(delta_time + delta_time / 100.0)
            & backwards_time.cmpgt(Vec3::splat(0.0));
        let resolution_axis = Vec3::select(valid_axes, backwards_time, Vec3::NAN).max_element();

        if properties.is_grounded.y && overlap.y > 0.0 && overlap.y < 0.51 {
            // This let's the player step up short distances when moving horizontally
            move_back.y = move_back.y.max(0.05_f32.min(overlap.y + overlap.y / 100.0));
            properties.is_grounded.y = true;
            properties.velocity.y = 0.0;

            if velocity.y.is_sign_positive() {
                *friction = friction.max(config.surface_friction(BlockFace::Bottom));
            } else {
                *friction = friction.max(config.surface_friction(BlockFace::Top));
            }
        } else if resolution_axis == backwards_time.y {
            move_back.y = overlap.y + overlap.y / 100.0;
            properties.is_grounded.y = true;
            properties.velocity.y = 0.0;

            if velocity.y.is_sign_positive() {
                *friction = friction.max(config.surface_friction(BlockFace::Bottom));
            } else {
                *friction = friction.max(config.surface_friction(BlockFace::Top));
            }
        } else if resolution_axis == backwards_time.x {
            move_back.x = overlap.x + overlap.x / 100.0;
            properties.is_grounded.x = true;
            properties.velocity.x = 0.0;

            if velocity.x.is_sign_positive() {
                *friction = friction.max(config.surface_friction(BlockFace::Left));
            } else {
                *friction = friction.max(config.surface_friction(BlockFace::Right));
            }
        } else if resolution_axis == backwards_time.z {
            move_back.z = overlap.z + overlap.z / 100.0;
            properties.is_grounded.z = true;
            properties.velocity.z = 0.0;

            if velocity.z.is_sign_positive() {
                *friction = friction.max(config.surface_friction(BlockFace::Back));
            } else {
                *friction = friction.max(config.surface_friction(BlockFace::Front));
            }
        } else {
            // When velocity is really small there's numerical precision problems. Since a
            // resolution is guaranteed. Move it back by whatever the smallest resolution
            // direction is.
            let valid_axes = Vec3::select(
                backwards_time.cmpgt(Vec3::ZERO) & backwards_time.cmplt(delta_time * 2.0),
                backwards_time,
                Vec3::NAN,
            );
            if valid_axes.x.is_finite() || valid_axes.y.is_finite() || valid_axes.z.is_finite() {
                let valid_axes = Vec3::select(
                    valid_axes.cmpeq(Vec3::splat(valid_axes.min_element())),
                    valid_axes,
                    Vec3::ZERO,
                );
                *move_back += (valid_axes + valid_axes / 100.0) * -velocity;
            }
        }
    }
}

/// An Axis-Aligned Bounding Box
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Aabb {
    pub center: Vec3,
    pub half_extents: Vec3,
}

impl Aabb {
    #[inline]
    pub fn from_min_max(min: Vec3, max: Vec3) -> Self {
        let center = 0.5 * (max + min);
        let half_extents = 0.5 * (max - min);
        Self {
            center,
            half_extents,
        }
    }

    #[inline]
    pub fn min(&self) -> Vec3 {
        self.center - self.half_extents
    }

    #[inline]
    pub fn max(&self) -> Vec3 {
        self.center + self.half_extents
    }

    #[inline]
    pub fn intersection(&self, other: &Self) -> Option<Vec3> {
        let distance = self.center - other.center;
        let overlap = self.half_extents + other.half_extents - distance.abs();

        if overlap.cmpgt(Vec3::ZERO).all() {
            // Keep sign to differentiate which side of the block was collided with.
            Some(overlap.copysign(distance))
        } else {
            None
        }
    }

    /// Transforms the aabb by first rotating and scaling it and then applying the translation.
    pub fn transform(&self, transform: &Transform) -> Self {
        let rot_mat = Mat3::from_quat(transform.rotation.as_quat());
        // If you rotate a square normally, its aabb will grow larger at 45 degrees because the
        // diagonal of the square is longer and pointing in the axis direction. We don't want
        // our aabbs to grow larger, we want a constant volume because they are easier to deal with
        // in physics. Lets us use uniform aabbs without worrying about contortions.
        //
        // let abs_rot_mat = DMat3::from_cols(
        //     rot_mat.x_axis.abs(),
        //     rot_mat.y_axis.abs(),
        //     rot_mat.z_axis.abs(),
        // );
        //
        // This is how you do it normally, each column will have a euclidean distance of 1. At a 45
        // degree angle around the y axis, this will give an x_axis of
        // [sqrt(2)/2=0.707, 0.0, 0.707], i.e. take 70% of the x extent and 70% of the z
        // extent. We want it to only take 50%. This is done by normalizing it so its total
        // sum is 1.
        let abs_rot_mat = Mat3::from_cols(
            rot_mat.x_axis.abs() / rot_mat.x_axis.abs().element_sum(),
            rot_mat.y_axis.abs() / rot_mat.y_axis.abs().element_sum(),
            rot_mat.z_axis.abs() / rot_mat.z_axis.abs().element_sum(),
        );

        Self {
            center: rot_mat * self.center * transform.scale + transform.translation,
            half_extents: abs_rot_mat * self.half_extents * transform.scale,
        }
    }
}

#[derive(Deserialize)]
pub struct CollisionConfig {
    collider: Collider,
    friction: Friction,
    climbable: bool,
}

impl CollisionConfig {
    fn surface_friction(&self, face: BlockFace) -> Vec3 {
        let friction = match self.friction {
            Friction::Surface {
                front,
                back,
                right,
                left,
                top,
                bottom,
            } => match face {
                BlockFace::Front => Vec3::splat(front),
                BlockFace::Back => Vec3::splat(back),
                BlockFace::Right => Vec3::splat(right),
                BlockFace::Left => Vec3::splat(left),
                BlockFace::Top => Vec3::splat(top),
                BlockFace::Bottom => Vec3::splat(bottom),
            },
            _ => return Vec3::ZERO,
        };

        friction
    }

    fn drag(&self) -> Option<Vec3> {
        match self.friction {
            Friction::Drag(drag) => return Some(drag),
            _ => return None,
        }
    }
}

enum BlockFace {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Deserialize)]
pub enum Friction {
    Surface {
        front: f32,
        back: f32,
        right: f32,
        left: f32,
        top: f32,
        bottom: f32,
    },
    Drag(Vec3),
}

#[derive(Deserialize)]
pub enum Collider {
    Single(Aabb),
    Multi(Vec<Aabb>),
}

impl Collider {
    #[inline]
    fn iter(&self) -> std::slice::Iter<'_, Aabb> {
        match self {
            Collider::Single(aabb) => std::slice::from_ref(aabb).iter(),
            Collider::Multi(aabbs) => aabbs.iter(),
        }
    }

    fn as_aabb(&self) -> Aabb {
        match self {
            Self::Single(aabb) => aabb.clone(),
            Self::Multi(aabbs) => {
                let mut min = Vec3::MAX;
                let mut max = Vec3::MIN;
                for aabb in aabbs {
                    min = min.min(aabb.min());
                    max = max.max(aabb.max());
                }

                Aabb::from_min_max(min, max)
            }
        }
    }

    /// Iterator over the block positions inside the collider
    fn iter_block_positions(&self, transform: &Transform) -> impl IntoIterator<Item = IVec3> {
        let aabb = self.as_aabb().transform(transform);

        let min = aabb.min().floor().as_ivec3();
        let max = aabb.max().floor().as_ivec3();
        (min.x..=max.x).flat_map(move |x| {
            (min.z..=max.z).flat_map(move |z| (min.y..=max.y).map(move |y| IVec3::new(x, y, z)))
        })
    }

    /// Intersection test with another collider, returns the overlap if any.
    #[inline]
    fn intersection(
        &self,
        self_transform: &Transform,
        other_transform: &Transform,
        other: &Collider,
    ) -> Option<Vec3> {
        let mut intersection = Vec3::ZERO;

        for left_aabb in self.iter() {
            let left_aabb = left_aabb.transform(self_transform);
            for right_aabb in other.iter().map(|aabb| aabb.transform(other_transform)) {
                if let Some(new_intersection) = left_aabb.intersection(&right_aabb) {
                    intersection = intersection
                        .abs()
                        .max(new_intersection.abs())
                        .copysign(new_intersection);
                }
            }
        }

        if intersection != Vec3::ZERO {
            return Some(intersection);
        } else {
            return None;
        }
    }
}

pub struct BlockState(pub u16);

impl BlockState {
    fn rotation(&self) -> DQuat {
        if self.0 & 0b100 == 0 {
            match self.0 & 0b11 {
                0 => DQuat::from_rotation_y(0.0),
                1 => DQuat::from_rotation_y(std::f64::consts::FRAC_PI_2),
                2 => DQuat::from_rotation_y(std::f64::consts::PI),
                3 => DQuat::from_rotation_y(-std::f64::consts::FRAC_PI_2),
                _ => unreachable!(),
            }
        } else {
            return DQuat::IDENTITY;
        }
    }
}
