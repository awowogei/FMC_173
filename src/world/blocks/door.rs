use fmc::{
    bevy::math::DQuat,
    blocks::{BlockData, Blocks},
    prelude::*,
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
    mut block_hits: Query<
        (Entity, &mut Door, &mut HandInteractions, &mut Transform),
        Changed<HandInteractions>,
    >,
) {
    for (_entity, mut door, mut interactions, mut transform) in block_hits.iter_mut() {
        for _interaction in interactions.read() {
            if door.open {
                transform.rotate(DQuat::from_rotation_y(-std::f64::consts::FRAC_PI_2));
            } else {
                transform.rotate(DQuat::from_rotation_y(std::f64::consts::FRAC_PI_2));
            }
            door.open = !door.open;
        }
    }
}
