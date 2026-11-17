use std::time::Duration;

use fmc::{
    commands::{AddCommand, Command, CommandSystems},
    networking::Server,
    prelude::*,
    protocol::messages,
};

/// Handles the day/night cycle
pub struct SkyPlugin;
impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Clock {
            time: Duration::from_secs_f32(100.0),
        })
        .add_command::<TimeCommand>()
        .add_systems(
            Update,
            (
                day_night_cycle,
                handle_time_command.after(CommandSystems::Dispatch),
            ),
        );
    }
}

// time = 0, dawn
// time = 600, dusk
const DAY_LENGTH: f32 = 1200.0;
const SUNRISE: f32 = 0.0;
const SUNSET: f32 = DAY_LENGTH / 2.0;
const MIDNIGHT: f32 = DAY_LENGTH * 0.75;
const NOON: f32 = DAY_LENGTH * 0.25;

/// The current time of day, 0s = dawn, 600s = dusk
#[derive(Resource)]
pub struct Clock {
    time: Duration,
}

impl Clock {
    pub fn is_night(&self) -> bool {
        self.time.as_secs() > 700 && self.time.as_secs() < 1100
    }

    pub fn set_time(&mut self, time: f32) {
        // rem_euclid is just modulo that wraps around when the time is negative. e.g. -10
        // gives DAY_LENGTH - 10 instead of just -10
        self.time = Duration::from_secs_f32(time.rem_euclid(DAY_LENGTH));
    }

    pub fn get_time(&self) -> f32 {
        self.time.as_secs_f32() % DAY_LENGTH
    }

    pub fn set_sunrise(&mut self) {
        self.time = Duration::from_secs_f32(SUNRISE);
    }

    pub fn set_sunset(&mut self) {
        self.time = Duration::from_secs_f32(SUNSET);
    }

    pub fn set_noon(&mut self) {
        self.time = Duration::from_secs_f32(NOON);
    }

    pub fn set_midnight(&mut self) {
        self.time = Duration::from_secs_f32(MIDNIGHT);
    }
}

fn day_night_cycle(time: Res<Time>, net: Res<Server>, mut clock: ResMut<Clock>) {
    clock.time += time.delta();

    let message = messages::Time {
        angle: clock.time.as_secs_f32() * std::f32::consts::TAU / DAY_LENGTH,
    };

    net.broadcast(message);
}

#[derive(Message)]
enum TimeCommand {
    Noon,
    Night,
    Dawn,
    Dusk,
    Custom(f32),
}

impl Command for TimeCommand {
    const NAME: &'static str = "time";
    const USAGE: &'static str = "time <noon|night|dawn|dusk|0-1200> - Set the time of day";

    fn parse(_source: Entity, args: &[&str]) -> Result<Self, String> {
        let [time] = args else {
            return Err(Self::USAGE.to_owned());
        };

        match *time {
            "noon" => Ok(Self::Noon),
            "night" => Ok(Self::Night),
            "dawn" => Ok(Self::Dawn),
            "dusk" => Ok(Self::Dusk),
            custom_time => match custom_time.parse::<u32>() {
                Ok(time) if time <= 1200 => Ok(Self::Custom(time as f32)),
                Ok(_) => Err("Time must be between 0 and 1200".to_owned()),
                Err(_) => Err(
                    "Invalid time, use noon, night, dawn, dusk or a number from 0 to 1200"
                        .to_owned(),
                ),
            },
        }
    }
}

fn handle_time_command(mut clock: ResMut<Clock>, mut time_commands: MessageReader<TimeCommand>) {
    for time_command in time_commands.read() {
        match time_command {
            TimeCommand::Noon => clock.set_noon(),
            TimeCommand::Night => clock.set_midnight(),
            TimeCommand::Dawn => clock.set_sunrise(),
            TimeCommand::Dusk => clock.set_sunset(),
            TimeCommand::Custom(time) => clock.set_time(*time),
        }
    }
}
