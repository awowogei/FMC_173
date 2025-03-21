use std::collections::BinaryHeap;

use fmc::{
    bevy::math::{DVec2, DVec3},
    blocks::{BlockPosition, Blocks, Friction},
    prelude::*,
    world::WorldMap,
};
use indexmap::{map::Entry, IndexMap};

#[derive(Component)]
pub struct PathFinder {
    entity_width: usize,
    entity_height: usize,
    start: BlockPosition,
    goal: BlockPosition,
    path: Vec<DVec3>,
    // Holds indices into the path of the most direct path to the goal
    shortcuts: Vec<DVec3>,
}

impl PathFinder {
    pub fn new(width: usize, height: usize) -> Self {
        return Self {
            entity_width: width,
            entity_height: height,
            start: BlockPosition::default(),
            goal: BlockPosition::default(),
            path: Vec::new(),
            shortcuts: Vec::new(),
        };
    }

    pub fn find_path(&mut self, world_map: &WorldMap, start: DVec3, goal: DVec3) {
        let block_start = BlockPosition::from(start);
        let block_goal = BlockPosition::from(goal);
        if block_start != block_goal && self.goal != block_goal {
            self.start = block_start;
            self.goal = block_goal;
        } else {
            return;
        }

        self.path.clear();
        self.shortcuts.clear();

        let mut queue = BinaryHeap::with_capacity(16_usize.pow(3));
        let mut node_map = IndexMap::new();
        node_map.insert(
            block_start,
            PathNode {
                parent_index: usize::MAX,
                cost: 0.0,
            },
        );

        let mut potential_successors = Vec::new();

        queue.push(Successor {
            node_index: 0,
            move_cost: 0.0,
            heuristic_cost: f32::MAX,
        });

        // Limit to how many steps it can take to circumvent obstacles
        let mut roundabout_limit = 0;

        let mut best_node_index = 0;
        let mut best_node_cost = f32::MAX;

        while let Some(successor) = queue.pop() {
            let (node_position, path_node) = node_map.get_index(successor.node_index).unwrap();

            if successor.cost() < best_node_cost {
                best_node_cost = successor.cost();
                best_node_index = successor.node_index;
            } else {
                roundabout_limit += 1;
            }

            if roundabout_limit > 25 {
                self.set_path(best_node_index, &node_map, None);
                return;
            }

            if successor.cost() > path_node.cost && path_node.parent_index != usize::MAX {
                continue;
            }

            potential_successors.clear();
            self.get_potential_successors(node_position, world_map, &mut potential_successors);

            for (position, move_cost, heuristic_cost) in potential_successors.drain(..) {
                let cost = successor.move_cost + move_cost + heuristic_cost;
                let node_index;

                match node_map.entry(position) {
                    Entry::Occupied(mut e) => {
                        if e.get().cost > cost {
                            node_index = e.index();
                            e.insert(PathNode {
                                parent_index: successor.node_index,
                                cost,
                            });
                        } else {
                            continue;
                        }
                    }
                    Entry::Vacant(v) => {
                        node_index = v.index();
                        v.insert(PathNode {
                            parent_index: successor.node_index,
                            cost,
                        });
                    }
                }

                if position == block_goal {
                    self.set_path(node_index, &node_map, Some(goal));
                    return;
                }

                queue.push(Successor {
                    node_index,
                    move_cost: successor.move_cost + move_cost,
                    heuristic_cost,
                });
            }
        }
    }

    pub fn next_node(&mut self, current_postition: DVec3) -> Option<DVec3> {
        while let Some(next_position) = self.path.last() {
            if next_position.xz().distance_squared(current_postition.xz()) >= 0.25 {
                return Some(*next_position);
            }

            self.path.pop();
        }
        None
    }

    pub fn next_shortcut(&mut self, current_postition: DVec3) -> Option<DVec3> {
        while let Some(next_position) = self.shortcuts.last() {
            if next_position.xz().distance_squared(current_postition.xz()) >= 0.25 {
                return Some(*next_position);
            }

            self.shortcuts.pop();
        }
        None
    }

    fn get_move_cost(world_map: &WorldMap, position: BlockPosition) -> f32 {
        if let Some(block_id) = world_map.get_block(position) {
            let block_config = Blocks::get().get_config(&block_id);
            if block_config.is_solid() {
                f32::INFINITY
            } else {
                block_config.drag.max_element() as f32
            }
        } else {
            f32::INFINITY
        }
    }

    fn get_heuristic_cost(&self, position: BlockPosition) -> f32 {
        position.distance_squared(*self.goal).abs() as f32
        //let delta = (position - self.goal).abs().as_vec3();

        //return delta.x + delta.y + delta.z;
        //if dx > 0 {
        //    dx = (dx - self.entity_width as i32 + 1).max(0);
        //}
        //if dy > 0 {
        //    dy = (dy - self.entity_height as i32 + 1).max(0);
        //}
        //if dz > 0 {
        //    dz = (dz - self.entity_width as i32 + 1).max(0);
        //}
        //
        //dx = dx.abs();
        //dy = dy.abs();
        //dz = dz.abs();
        //
        //let min = dx.min(dz) as f32;
        //
        //let diagonal = std::f32::consts::SQRT_2 * min;
        //let direct = (dx + dz) as f32 - min * 2.0;
        //let vertical = dy as f32 * 0.5;
        //
        //return diagonal + direct + vertical;
    }

    fn get_potential_successors(
        &self,
        position: &BlockPosition,
        world_map: &WorldMap,
        successors: &mut Vec<(BlockPosition, f32, f32)>,
    ) {
        let above_cost = Self::get_move_cost(world_map, *position + IVec3::Y);

        let max_steps = if above_cost == f32::INFINITY { 0 } else { 1 };

        let get_successor = |offset: IVec3| -> (BlockPosition, f32, f32) {
            let mut position = *position + offset;

            let mut move_cost = Self::get_move_cost(world_map, position);

            if move_cost == f32::INFINITY && max_steps == 1 {
                // Hit a wall, try to jump up
                position += IVec3::new(0, 1, 0);
                move_cost = Self::get_move_cost(world_map, position);
                (position, move_cost + 1.0, self.get_heuristic_cost(position))
            } else {
                let mut steps = 0;
                loop {
                    if steps > 2 {
                        return (position, f32::INFINITY, f32::INFINITY);
                    }

                    let below_cost =
                        Self::get_move_cost(world_map, position - IVec3::new(0, steps + 1, 0));
                    if below_cost == f32::INFINITY {
                        move_cost += steps as f32;

                        position -= IVec3::new(0, steps, 0);
                        return (position, move_cost, self.get_heuristic_cost(position));
                    }

                    move_cost += below_cost;
                    steps += 1;
                }
            }
        };

        for offset in [
            IVec3::X,
            IVec3::NEG_X,
            IVec3::Z,
            IVec3::NEG_Z,
            IVec3::X + IVec3::Z,
            IVec3::X - IVec3::Z,
            -IVec3::X + IVec3::Z,
            -IVec3::X - IVec3::Z,
        ] {
            let (node_position, move_cost, heuristic_cost) = get_successor(offset);
            if move_cost != f32::INFINITY {
                successors.push((node_position, move_cost, heuristic_cost));
            }
        }
    }

    fn set_path(
        &mut self,
        mut index: usize,
        node_map: &IndexMap<BlockPosition, PathNode>,
        accurate_goal: Option<DVec3>,
    ) {
        let mut xz_offset = DVec3::splat(self.entity_width as f64 * 0.5);
        xz_offset.y = 0.0;

        while index != usize::MAX {
            let (position, path_node) = node_map.get_index(index).unwrap();

            self.path.push(position.as_dvec3() + xz_offset);
            index = path_node.parent_index;
        }

        // Since the goal is first reduced to a block position for pathfinding it needs to be
        // swapped out with the correct value. When the accurate goal is None, it's a best guess
        // path.
        if let Some(accurate_goal) = accurate_goal {
            self.path[0] = accurate_goal;
        }
        // Same, but since the mob will already be at the start position, it can be removed.
        self.path.pop();

        self.calc_shortcuts();
    }

    fn calc_shortcuts(&mut self) {
        let start = self.start.as_dvec3();
        let goal = self.path[0];
        let mut distance = goal.distance_squared(start);

        for position in self.path.iter().rev() {
            let new_distance = position.distance_squared(goal);
            // If it ever moves in a direction where the distance to the goal increases, it must
            // mean it's pathing around something.
            if new_distance > distance {
                self.shortcuts.push(*position);
            }
            distance = new_distance;
        }

        if self.shortcuts.len() > self.path.len() {
            panic!();
        }

        // The goal
        self.shortcuts.push(goal);
        self.shortcuts.reverse();
    }
}

// The current best cost of the path to a node
struct PathNode {
    parent_index: usize,
    cost: f32,
}

// A potential new best path node
struct Successor {
    node_index: usize,
    // The cumulative move cost of the node
    move_cost: f32,
    // The node's distance from the goal
    heuristic_cost: f32,
}

impl Successor {
    fn cost(&self) -> f32 {
        self.move_cost + self.heuristic_cost
    }
}

impl Ord for Successor {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match other.cost().total_cmp(&self.cost()) {
            std::cmp::Ordering::Equal => other.heuristic_cost.total_cmp(&self.heuristic_cost),
            ordering => ordering,
        }
    }
}

impl PartialEq for Successor {
    fn eq(&self, other: &Self) -> bool {
        self.move_cost.eq(&other.move_cost) && self.heuristic_cost.eq(&other.heuristic_cost)
    }
}

impl Eq for Successor {}

impl PartialOrd for Successor {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
