use std::time::Duration;

use fmc::{networking::Server, prelude::*, protocol::messages};

/// Handles the day/night cycle
pub struct SkyPlugin;
impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Clock::default())
            .add_systems(Update, day_night_cycle);
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
#[derive(Default, DerefMut, Deref, Resource)]
pub struct Clock {
    time: Duration,
}

impl Clock {
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
