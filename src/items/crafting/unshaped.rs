use fmc::items::{ItemId, ItemStack};

#[derive(Hash, PartialEq, Eq)]
pub struct Pattern {
    pub(super) inner: Vec<ItemId>,
}

impl From<&[ItemStack]> for Pattern {
    fn from(grid: &[ItemStack]) -> Self {
        let mut inner: Vec<ItemId> = grid
            .iter()
            .filter_map(|item_stack| item_stack.item().map(|item| item.id))
            .collect();
        inner.sort();

        return Pattern { inner };
    }
}

pub struct Recipe {
    pub(super) required_amount: Vec<u32>,
    pub(super) output: ItemStack,
}

fn sorted_indices(input: &[ItemStack]) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..input.len())
        .filter(|&index| !input[index].is_empty())
        .collect();
    indices.sort_by_key(|&index| input[index].item().map(|item| item.id));
    return indices;
}

impl Recipe {
    pub(super) fn craft(&self, input: &mut [ItemStack], mut amount: u32) -> Option<ItemStack> {
        amount = std::cmp::min(
            amount / self.output.size(),
            self.get_craftable_amount(input) / self.output.size(),
        );

        if amount == 0 {
            return None;
        }

        for (index, required) in sorted_indices(input).into_iter().zip(&self.required_amount) {
            input[index].take(required * amount);
        }

        let mut output = self.output.clone();
        output.set_size(amount * self.output.size());

        return Some(output);
    }

    pub(super) fn get_craftable_amount(&self, input: &[ItemStack]) -> u32 {
        let amount_can_craft = sorted_indices(input)
            .into_iter()
            .zip(&self.required_amount)
            .map(|(index, required)| input[index].size() / required)
            .min();

        return match amount_can_craft {
            Some(amount) => amount * self.output.size(),
            None => 0,
        };
    }

    pub fn output(&self) -> &ItemStack {
        return &self.output;
    }
}
