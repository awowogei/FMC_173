use fmc::{players::Player, prelude::*};

use crate::players::{HealEvent, Inventory};

use super::{ItemAction, ItemEvents, Items};

pub struct BreadPlugin;
impl Plugin for BreadPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, register_bread)
            .add_systems(Update, eat_bread.after(super::ItemSystems));
    }
}

#[derive(Component)]
struct Bread;

fn register_bread(mut commands: Commands, items: Res<Items>) {
    let item_id = items.get_id("bread").unwrap();
    commands.spawn((ItemEvents::new(item_id), Bread));
}

fn eat_bread(
    mut bread_query: Query<&mut ItemEvents, (With<Bread>, Changed<ItemEvents>)>,
    mut player_query: Query<&mut Inventory, With<Player>>,
    mut heal_events: MessageWriter<HealEvent>,
) {
    let Ok(mut events) = bread_query.single_mut() else {
        return;
    };

    for event in events.read() {
        if event.action != ItemAction::Use {
            continue;
        }

        let mut inventory = player_query.get_mut(event.player_entity).unwrap();
        let held_item = inventory.held_item_stack_mut();

        heal_events.write(HealEvent {
            player_entity: event.player_entity,
            healing: 8,
        });

        held_item.pop();
    }
}
