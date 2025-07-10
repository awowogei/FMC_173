use fmc::{
    bevy::ecs::system::EntityCommands,
    blocks::{BlockData, BlockPosition, Blocks},
    prelude::*,
    world::BlockUpdate,
};

pub struct WheatPlugin;
impl Plugin for WheatPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup).add_systems(Update, grow);
    }
}

#[derive(Component)]
struct Wheat {
    stage: u8,
    grow_timer: Timer,
}

impl Default for Wheat {
    fn default() -> Self {
        Self {
            stage: 0,
            grow_timer: Timer::from_seconds(1.0, TimerMode::Repeating),
        }
    }
}

fn setup(mut blocks: ResMut<Blocks>) {
    let block_id = blocks.get_id("wheat_0");
    let block = blocks.get_config_mut(&block_id);
    block.set_spawn_function(spawn_wheat_0);

    let block_id = blocks.get_id("wheat_1");
    let block = blocks.get_config_mut(&block_id);
    block.set_spawn_function(spawn_wheat_1);

    let block_id = blocks.get_id("wheat_2");
    let block = blocks.get_config_mut(&block_id);
    block.set_spawn_function(spawn_wheat_2);

    let block_id = blocks.get_id("wheat_3");
    let block = blocks.get_config_mut(&block_id);
    block.set_spawn_function(spawn_wheat_3);

    let block_id = blocks.get_id("wheat_4");
    let block = blocks.get_config_mut(&block_id);
    block.set_spawn_function(spawn_wheat_4);

    let block_id = blocks.get_id("wheat_5");
    let block = blocks.get_config_mut(&block_id);
    block.set_spawn_function(spawn_wheat_5);

    let block_id = blocks.get_id("wheat_6");
    let block = blocks.get_config_mut(&block_id);
    block.set_spawn_function(spawn_wheat_6);
}

fn spawn_wheat_0(commands: &mut EntityCommands, _block_data: Option<&BlockData>) {
    commands.insert(Wheat {
        stage: 0,
        ..default()
    });
}
fn spawn_wheat_1(commands: &mut EntityCommands, _block_data: Option<&BlockData>) {
    commands.insert(Wheat {
        stage: 1,
        ..default()
    });
}
fn spawn_wheat_2(commands: &mut EntityCommands, _block_data: Option<&BlockData>) {
    commands.insert(Wheat {
        stage: 2,
        ..default()
    });
}
fn spawn_wheat_3(commands: &mut EntityCommands, _block_data: Option<&BlockData>) {
    commands.insert(Wheat {
        stage: 3,
        ..default()
    });
}
fn spawn_wheat_4(commands: &mut EntityCommands, _block_data: Option<&BlockData>) {
    commands.insert(Wheat {
        stage: 4,
        ..default()
    });
}
fn spawn_wheat_5(commands: &mut EntityCommands, _block_data: Option<&BlockData>) {
    commands.insert(Wheat {
        stage: 5,
        ..default()
    });
}
fn spawn_wheat_6(commands: &mut EntityCommands, _block_data: Option<&BlockData>) {
    commands.insert(Wheat {
        stage: 6,
        ..default()
    });
}

// TODO: Make 'tick' increment randomly.
// TODO: Only run this function at daytime?
fn grow(
    time: Res<Time>,
    mut growing: Query<(&mut Wheat, &BlockPosition)>,
    mut block_update_writer: EventWriter<BlockUpdate>,
) {
    for (mut wheat, block_position) in growing.iter_mut() {
        if wheat.stage == 6 {
            continue;
        }

        wheat.grow_timer.tick(time.delta());
        if !wheat.grow_timer.just_finished() {
            continue;
        }

        wheat.stage += 1;

        let blocks = Blocks::get();
        let block_id = match wheat.stage {
            0 => blocks.get_id("wheat_1"),
            1 => blocks.get_id("wheat_2"),
            2 => blocks.get_id("wheat_3"),
            3 => blocks.get_id("wheat_4"),
            4 => blocks.get_id("wheat_5"),
            5 => blocks.get_id("wheat_6"),
            6 => blocks.get_id("wheat_7"),
            _ => unreachable!(),
        };

        block_update_writer.write(BlockUpdate::Swap {
            position: *block_position,
            block_id,
            block_state: None,
        });
    }
}
