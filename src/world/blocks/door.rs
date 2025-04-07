use fmc::{
    bevy::math::{DQuat, DVec3},
    blocks::{BlockData, BlockPosition, BlockRotation, Blocks},
    prelude::*,
    world::WorldMap,
};

use crate::players::HandInteractions;

pub struct DoorPlugin;
impl Plugin for DoorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(Update, handle_block_hits);
    }
}

#[derive(Component)]
struct Door {
    open: bool,
}

fn setup(mut blocks: ResMut<Blocks>) {
    let block_id = blocks.get_id("oak door");
    let block = blocks.get_config_mut(&block_id);
    block.set_spawn_function(spawn_function);
}

fn spawn_function(commands: &mut EntityCommands, _block_data: Option<&BlockData>) {
    commands.insert((Door { open: false }, HandInteractions::default()));
}

fn handle_block_hits(
    world_map: Res<WorldMap>,
    mut block_hits: Query<
        (
            Entity,
            &mut Door,
            &mut HandInteractions,
            &mut Transform,
            &BlockPosition,
        ),
        Changed<HandInteractions>,
    >,
) {
    for (_entity, mut door, mut interactions, mut transform, block_position) in
        block_hits.iter_mut()
    {
        let Some(block_state) = world_map.get_block_state(*block_position) else {
            continue;
        };

        for _interaction in interactions.read() {
            let inset = 1.0 / 16.0;
            let offset = match block_state.rotation().unwrap() {
                BlockRotation::Front => DVec3::new(1.0, 0.0, 0.0) + DVec3::new(-inset, 0.0, inset),
                BlockRotation::Right => DVec3::new(0.0, 0.0, 0.0) + DVec3::new(inset, 0.0, inset),
                BlockRotation::Back => DVec3::new(0.0, 0.0, 1.0) + DVec3::new(inset, 0.0, -inset),
                BlockRotation::Left => DVec3::new(1.0, 0.0, 1.0) + DVec3::new(-inset, 0.0, -inset),
            };
            // The divide by 16 part is because we want to rotate around the point that is one
            // pixel deep into the door.
            let corner = block_position.as_dvec3() + offset;
            if door.open {
                transform
                    .rotate_around(corner, DQuat::from_rotation_y(-std::f64::consts::FRAC_PI_2));
            } else {
                transform
                    .rotate_around(corner, DQuat::from_rotation_y(std::f64::consts::FRAC_PI_2));
            }
            door.open = !door.open;
        }
    }
}
