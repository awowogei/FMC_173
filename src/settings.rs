use fmc::{database::Database, prelude::*, terminal::Cli};

use serde::{Deserialize, Serialize};
use std::{
    hash::{DefaultHasher, Hasher},
    io::{BufRead, BufReader},
};

use crate::players::GameMode;

pub struct SettingsPlugin;
impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        let mut settings = if let Some(world_path) = Cli::world_path() {
            Settings::load_from_database(world_path).unwrap_or(Settings::default())
        } else {
            let mut settings = Settings::load_from_file();

            let database_path = if let Some(name) = &settings.world_name {
                name.clone()
            } else {
                Database::DEFAULT_PATH.to_owned()
            };

            // Overwrite settings from the file that can't be changed after the world
            // has been created.
            if let Some(db_settings) = Settings::load_from_database(&database_path) {
                settings.seed = db_settings.seed;
            }

            // Different database path so we override the database
            app.insert_resource(Database::new(database_path));

            settings
        };

        if settings.seed.is_empty() {
            settings.seed = std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                .to_string();
        }

        app.insert_resource(settings);

        app.add_systems(Update, save_settings.run_if(resource_changed::<Settings>));
    }
}

/// Global settings
#[derive(Resource, Deserialize, Serialize)]
#[serde(default)]
pub struct Settings {
    // Chooses which world to load when the settings are loaded from the settings file.
    world_name: Option<String>,
    // World seed
    seed: String,
    /// Pvp enabled
    pub pvp: bool,
    /// The max render distance
    pub render_distance: u32,
    /// The default game mode of new players
    pub game_mode: GameMode,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            world_name: None,
            seed: "".to_owned(),
            pvp: false,
            render_distance: 16,
            game_mode: GameMode::Survival,
        }
    }
}

impl Settings {
    fn load_from_database(path: &str) -> Option<Self> {
        let Ok(connection) = rusqlite::Connection::open(path) else {
            return None;
        };

        let Ok(settings_json) = connection.query_row(
            "SELECT data FROM storage WHERE name='settings'",
            [],
            |row| row.get::<usize, String>(0),
        ) else {
            return None;
        };

        let settings = match serde_json::from_str(&settings_json) {
            Ok(s) => s,
            Err(e) => {
                error!("{e}");
                return None;
            }
        };

        settings
    }

    fn load_from_file() -> Self {
        let mut settings = Settings::default();

        let path = "./server_settings.txt";
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(_) => {
                Self::write_template();
                return settings;
            }
        };
        let reader = BufReader::new(file);

        for (line_num, line) in reader.lines().enumerate() {
            let line = line.unwrap();

            // comments
            if line.starts_with("#") {
                continue;
            }

            let (name, value) = line.split_once("=").unwrap_or_else(|| {
                panic!(
                    "Error reading server_settings.txt at line {line_num}. \
                    All settings must be of the format 'name = setting', it cannot be: '{line}'",
                );
            });
            let name = name.trim();
            let value = value.trim();

            match name {
                "world-name" => {
                    if value.is_empty() {
                        panic!("The world name cannot be empty");
                    }
                    settings.world_name = Some("./".to_owned() + value + ".sqlite");
                }
                "seed" => {
                    settings.seed = value.to_owned();
                }
                "pvp" => {
                    settings.pvp = value.parse::<bool>().unwrap_or_else(|_| {
                        panic!(
                            "Server property 'pvp' must be one of 'true/false', cannot be: '{value}'",
                        )
                    });
                }
                "render-distance" => {
                    settings.render_distance = value.parse::<u32>().unwrap_or_else(|_| {
                        panic!(
                            "Server property 'render-distance' must be a positive number, cannot be: '{value}'",
                        )
                    });
                }
                "game-mode" => {
                    settings.game_mode = match value {
                        "survival" => GameMode::Survival,
                        "creative" => GameMode::Creative,
                        "spectator" => GameMode::Spectator,
                        e => {
                            panic!(
                                "Server property 'game-mode' must be one of 'survival', 'creative' or 'spectator', cannot be: '{e}'",
                            )
                        }
                    };
                }
                _ => {
                    error!("Invalid setting '{name}' in settings file at line {line_num}",);
                }
            }
        }

        return settings;
    }

    #[rustfmt::skip]
    fn save_to_file(&self) {
        let mut contents = String::new();
        if let Some(world_name) = &self.world_name {
            contents = contents + "world-name = " + world_name + "\n";
        }
        contents = contents + "seed = " + &self.seed + "\n";
        contents = contents + "pvp = " + &self.pvp.to_string() + "\n";
        contents = contents + "render-distance = " + &self.render_distance.to_string();

        std::fs::write("./server_settings.txt", contents).unwrap();
    }

    fn save_to_database(&self, database: &Database) {
        let connection = database.get_write_connection();

        connection
            .execute(
                "INSERT OR REPLACE INTO storage (name, data) VALUES (?,?)",
                rusqlite::params!["settings", serde_json::to_string(self).unwrap()],
            )
            .unwrap();
    }

    pub fn seed(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        hasher.write(self.seed.as_bytes());
        hasher.finish()
    }

    // Writes a template config to the server directory.
    #[rustfmt::skip]
    fn write_template() {
        let settings = Self::default();
        let contents = String::new()
            + "#world-name = world" + "\n"
            + "pvp = " + &settings.pvp.to_string();

        std::fs::write("./server_settings.txt", contents).unwrap();
    }
}

fn save_settings(settings: Res<Settings>, database: Res<Database>) {
    settings.save_to_file();
    settings.save_to_database(&database);
}
