use crate::{
    fmc::{
        networking::{NetworkEvent, NetworkMessage, Server},
        players::Player,
        prelude::*,
        protocol::messages,
    },
    players::GameMode,
    skybox::Clock,
};

pub const CHAT_FONT_SIZE: f32 = 8.0;
pub const CHAT_TEXT_COLOR: &str = "#ffffff";

const HELP_TEXT: &[&str] = &[
    "------------------------- Help --------------------------",
    "/time <noon|night|dawn|dusk|0-1200> - Set the time of day",
    "/gamemode <0|1> - survival(0) or creative(1)",
];

pub struct ChatPlugin;
impl Plugin for ChatPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (handle_chat_messages, send_connection_messages));
    }
}

fn handle_chat_messages(
    net: Res<Server>,
    mut player_query: Query<(&Player, &mut GameMode)>,
    mut clock: ResMut<Clock>,
    mut chat_message_query: MessageReader<NetworkMessage<messages::InterfaceTextInput>>,
) {
    for chat_message in chat_message_query.read() {
        if &chat_message.interface_path != "chat/input" {
            continue;
        }

        let Ok((player, mut game_mode)) = player_query.get_mut(chat_message.player_entity) else {
            // TODO: Should probably disconnect
            continue;
        };

        // TODO: All commands are handled here until a proper system is figured out
        if chat_message.text.starts_with("/") {
            let send_message = |text: &str| {
                net.send_one(
                    chat_message.player_entity,
                    messages::InterfaceTextUpdate {
                        interface_path: "chat/history".to_owned(),
                        index: i32::MAX,
                        text: text.to_owned(),
                        font_size: CHAT_FONT_SIZE,
                        color: CHAT_TEXT_COLOR.to_owned(),
                    },
                );
            };

            if chat_message.text.trim() == "/help" {
                for line in HELP_TEXT {
                    send_message(line);
                }
            } else if let Some(time) = chat_message.text.strip_prefix("/time ") {
                match time.trim() {
                    "noon" => clock.set_noon(),
                    "night" => clock.set_midnight(),
                    "dawn" => clock.set_sunrise(),
                    "dusk" => clock.set_sunset(),
                    custom_time => match custom_time.parse::<u32>() {
                        Ok(time) if time <= 1200 => clock.set_time(time as f32),
                        Ok(_) => send_message("Time must be between 0 and 1200"),
                        Err(_) => send_message(
                            "Invalid time, use noon, night, dawn, dusk or a number from 0 to 1200",
                        ),
                    },
                }
            } else if let Some(mode) = chat_message.text.strip_prefix("/gamemode ") {
                match mode.trim() {
                    "0" => *game_mode = GameMode::Survival,
                    "1" => *game_mode = GameMode::Creative,
                    _ => send_message("Invalid game mode, use 0 or 1"),
                }
            } else {
                send_message("Invalid command, type /help for a list of commands");
            }
        } else {
            net.broadcast(messages::InterfaceTextUpdate {
                interface_path: "chat/history".to_owned(),
                index: i32::MAX,
                text: format!("[{}] {}", &player.username, &chat_message.text),
                font_size: CHAT_FONT_SIZE,
                color: CHAT_TEXT_COLOR.to_owned(),
            });
        }
    }
}

// TODO: Maybe players should be passed the chat history too.
// TODO: The "joined game" message sometimes shows for the player that joined. Intermitent problem,
// the message should arrive before the client finishes setup. In which case it should be
// discarded after two event buffer switches.
fn send_connection_messages(
    net: Res<Server>,
    player_query: Query<&Player>,
    mut network_events: MessageReader<NetworkEvent>,
) {
    for event in network_events.read() {
        match event {
            NetworkEvent::Connected { entity } => {
                let player = &player_query.get(*entity).unwrap();
                net.broadcast(messages::InterfaceTextUpdate {
                    interface_path: "chat/history".to_owned(),
                    index: i32::MAX,
                    text: format!("{} joined the game", player.username),
                    font_size: CHAT_FONT_SIZE,
                    color: CHAT_TEXT_COLOR.to_owned(),
                });
            }
            NetworkEvent::Disconnected { entity } => {
                let player = player_query.get(*entity).unwrap();
                net.broadcast(messages::InterfaceTextUpdate {
                    interface_path: "chat/history".to_owned(),
                    index: i32::MAX,
                    text: format!("{} left the game", player.username),
                    font_size: CHAT_FONT_SIZE,
                    color: CHAT_TEXT_COLOR.to_owned(),
                });
            }
        }
    }
}
