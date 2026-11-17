use crate::fmc::{
    commands::{
        AddCommand, Command, CommandError, CommandErrorEvent, CommandEvent, CommandRegistry,
        CommandSystems,
    },
    networking::{NetworkEvent, NetworkMessage, Server},
    permissions::{Permissions, Roles},
    players::Player,
    prelude::*,
    protocol::messages,
};

pub const CHAT_FONT_SIZE: f32 = 8.0;
pub const CHAT_TEXT_COLOR: &str = "#ffffff";

pub struct ChatPlugin;
impl Plugin for ChatPlugin {
    fn build(&self, app: &mut App) {
        app.add_command::<HelpCommand>().add_systems(
            Update,
            (
                handle_chat_messages.before(CommandSystems::Dispatch),
                (handle_help_command, send_command_errors).after(CommandSystems::Dispatch),
                send_connection_messages,
            ),
        );
    }
}

fn chat_line(text: impl Into<String>) -> messages::InterfaceTextUpdate {
    messages::InterfaceTextUpdate {
        interface_path: "chat/history".to_owned(),
        index: i32::MAX,
        text: text.into(),
        font_size: CHAT_FONT_SIZE,
        color: CHAT_TEXT_COLOR.to_owned(),
    }
}

fn handle_chat_messages(
    net: Res<Server>,
    player_query: Query<&Player>,
    mut command_events: MessageWriter<CommandEvent>,
    mut chat_message_query: MessageReader<NetworkMessage<messages::InterfaceTextInput>>,
) {
    for chat_message in chat_message_query.read() {
        if &chat_message.interface_path != "chat/input" {
            continue;
        }

        let Ok(player) = player_query.get(chat_message.player_entity) else {
            // TODO: Should probably disconnect
            continue;
        };

        let Some(command_text) = chat_message.text.strip_prefix("/") else {
            net.broadcast(chat_line(format!(
                "[{}] {}",
                &player.username, &chat_message.text
            )));
            continue;
        };

        command_events.write(CommandEvent {
            source: chat_message.player_entity,
            text: command_text.to_owned(),
        });
    }
}

#[derive(Message)]
struct HelpCommand {
    source: Entity,
    prefix: String,
}

impl Command for HelpCommand {
    const NAME: &'static str = "help";
    const USAGE: &'static str = "help [command] - List the available commands and their usage";

    fn parse(source: Entity, args: &[&str]) -> Result<Self, CommandError> {
        let prefix = match args {
            [] => String::new(),
            [prefix] => prefix.to_string(),
            _ => {
                return Err(CommandError::Malformed {
                    reason: "expected at most one argument".to_owned(),
                });
            }
        };

        Ok(Self { source, prefix })
    }
}

fn handle_help_command(
    net: Res<Server>,
    command_registry: Res<CommandRegistry>,
    permissions: Res<Permissions>,
    player_query: Query<Option<&Roles>, With<Player>>,
    mut help_commands: MessageReader<HelpCommand>,
) {
    for help_command in help_commands.read() {
        let Ok(roles) = player_query.get(help_command.source) else {
            continue;
        };

        net.send_one(
            help_command.source,
            chat_line("------------------------- Help --------------------------"),
        );
        for command in command_registry.query(&help_command.prefix) {
            let allowed = match command.permission {
                Some(permission) => {
                    roles.is_some_and(|roles| permissions.has_permission(roles, &permission))
                }
                None => true,
            };

            if allowed {
                net.send_one(
                    help_command.source,
                    chat_line(format!("/{}", command.usage)),
                );
            }
        }
    }
}

fn send_command_errors(
    net: Res<Server>,
    player_query: Query<(), With<Player>>,
    mut errors: MessageReader<CommandErrorEvent>,
) {
    for event in errors.read() {
        if player_query.contains(event.source) {
            let text = match &event.error {
                CommandError::Invalid { .. } => {
                    "Invalid command, type /help for a list of commands".to_owned()
                }
                CommandError::Malformed { reason } => format!("Malformed command: {reason}"),
                CommandError::Failed { reason } => reason.clone(),
            };

            net.send_one(event.source, chat_line(text));
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
