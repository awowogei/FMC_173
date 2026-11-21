use fmc::{
    commands::{AddCommand, Command, CommandError, CommandErrorEvent},
    items::ItemStack,
    physics::Physics,
    players::Player,
    prelude::*,
};

use crate::items::{DroppedItem, Items};

pub struct ItemCommandsPlugin;
impl Plugin for ItemCommandsPlugin {
    fn build(&self, app: &mut App) {
        app.add_command::<GiveCommand>()
            .add_systems(Update, handle_give_command);
    }
}

#[derive(Message)]
struct GiveCommand {
    source: Entity,
    item: String,
    count: u32,
    target: Option<String>,
}

impl Command for GiveCommand {
    const NAME: &'static str = "give";
    const USAGE: &'static str = "give <item> <count> [username] - Spawn an amount of an item";

    fn parse(source: Entity, args: &[&str]) -> Result<Self, CommandError> {
        let (item, count, target) = match args {
            [item, count] => (*item, *count, None),
            [item, count, target] => (*item, *count, Some(target.to_string())),
            _ => {
                return Err(CommandError::Malformed {
                    reason: "expected an item, a count and an optional username".to_owned(),
                });
            }
        };

        let count = match count.parse::<u32>() {
            Ok(count) if count > 0 => count,
            _ => {
                return Err(CommandError::Malformed {
                    reason: format!("invalid count '{count}', must be a positive number"),
                });
            }
        };

        Ok(Self {
            source,
            item: item.to_owned(),
            count,
            target,
        })
    }
}

fn handle_give_command(
    mut commands: Commands,
    items: Res<Items>,
    player_query: Query<(&Player, &GlobalTransform)>,
    mut give_commands: MessageReader<GiveCommand>,
    mut errors: MessageWriter<CommandErrorEvent>,
) {
    for give_command in give_commands.read() {
        let source = give_command.source;
        let fail = |reason: String| CommandErrorEvent {
            source,
            error: CommandError::Failed { reason },
        };

        let Some(item_id) = items.get_id(&give_command.item) else {
            errors.write(CommandErrorEvent {
                source,
                error: CommandError::Failed {
                    reason: format!("There is no item named '{}'", give_command.item),
                },
            });
            continue;
        };

        let target = match &give_command.target {
            Some(name) => player_query
                .iter()
                .find(|(player, _)| &player.username == name),
            None => player_query.get(source).ok(),
        };

        let Some((_, transform)) = target else {
            if let Some(name) = &give_command.target {
                errors.write(CommandErrorEvent {
                    source,
                    error: CommandError::Failed {
                        reason: format!("There is no player with the username '{name}'"),
                    },
                });
            }
            continue;
        };

        let position = transform.translation();
        commands.spawn((
            DroppedItem::new(ItemStack::new(item_id, give_command.count)).with_delay(0.0),
            Transform::from_translation(position),
            Physics::default(),
        ));
    }
}
