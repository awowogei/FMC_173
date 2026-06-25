use std::collections::{BinaryHeap, HashMap};

use fmc::{
    bevy::math::{DVec2, DVec3},
    blocks::{BlockPosition, Blocks},
    prelude::*,
    world::WorldMap,
};
use indexmap::{IndexMap, map::Entry};
use smallvec::SmallVec;

#[derive(Component)]
pub struct PathFinder {
    height: i32,
    width: i32,
    start: BlockPosition,
    goal: BlockPosition,
    previous_node: Option<DVec3>,
    path: Vec<DVec3>,
    jump_height: u32,
    movement_cost_cache: HashMap<BlockPosition, Option<f32>>,
}

impl PathFinder {
    pub fn new(height: u32, width: u32, jump_height: u32) -> Self {
        return Self {
            height: height as i32,
            width: width as i32,
            start: BlockPosition::default(),
            goal: BlockPosition::default(),
            previous_node: None,
            path: Vec::new(),
            jump_height,
            movement_cost_cache: HashMap::new(),
        };
    }

    pub fn has_goal(&self) -> bool {
        !self.path.is_empty()
    }

    pub fn goal(&self) -> Option<BlockPosition> {
        if !self.path.is_empty() {
            Some(self.goal)
        } else {
            None
        }
    }

    pub fn find_path(&mut self, world_map: &WorldMap, start: DVec3, goal: DVec3) {
        // Even width npcs walk the edges of the blocks while odd width npcs walk the center of blocks.
        let mut block_start = if self.width % 2 == 0 {
            BlockPosition::from(DVec3::new(start.x.round(), start.y, start.z.round()))
        } else {
            BlockPosition::from(start)
        };
        // The start position is the middle of the npc. To make working with it easier we
        // shift it to one of the corners
        block_start -= IVec3::new(self.width / 2, 0, self.width / 2);

        let block_goal = BlockPosition::from(goal);
        if block_start != block_goal && self.goal != block_goal {
            self.start = block_start;
            self.goal = block_goal;
        } else {
            return;
        }

        self.movement_cost_cache.clear();
        self.path.clear();

        // Direct paths feel much better, so we always try to find one before fallback to grid
        // based pathfinding.
        self.find_direct_path(world_map, start, goal);
        if !self.path.is_empty() {
            return;
        }

        let mut queue = BinaryHeap::with_capacity(100);
        let mut node_map = IndexMap::new();
        node_map.insert(
            block_start,
            PathNode {
                parent_index: usize::MAX,
                cost: 0.0,
            },
        );

        queue.push(Successor {
            node_index: 0,
            movement_cost: 0.0,
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

            for potential in self.get_potential_successors(node_position, world_map) {
                let cost =
                    successor.movement_cost + potential.movement_cost + potential.heuristic_cost;
                let node_index;

                match node_map.entry(potential.position) {
                    Entry::Occupied(mut entry) => {
                        if entry.get().cost > cost {
                            node_index = entry.index();
                            entry.insert(PathNode {
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

                if potential.position == block_goal {
                    self.set_path(node_index, &node_map, Some(goal));
                    return;
                }

                queue.push(Successor {
                    node_index,
                    movement_cost: successor.movement_cost + potential.movement_cost,
                    heuristic_cost: potential.heuristic_cost,
                });
            }
        }
    }

    // Try to find a straight path that leads directly to the goal. Will fail if there's any type
    // of obstruction.
    fn find_direct_path(&mut self, world_map: &WorldMap, start: DVec3, goal: DVec3) {
        let forward = (goal - start).normalize().xz();
        let direction = forward.signum();

        // How far along the forward vector you need to go to hit the next block in each direction.
        //
        // fract_gl() uses x - x.floor(), which yields the correct value for all negative
        // directions, e.g. fract_gl(-1.32) = 0.68. When the direction is positive it is just
        // inverted.
        let mut distance_next = start.xz().fract_gl();
        distance_next = DVec2::select(
            direction.cmpeq(DVec2::ONE),
            1.0 - distance_next,
            distance_next,
        );
        distance_next = distance_next / forward.abs();

        // How far along the forward vector you need to go to traverse one block in each direction.
        let distance_increment = 1.0 / forward.abs();
        // +/-1 to shift block_pos when it hits the grid
        let step = direction.as_ivec2();

        let mut block_position = BlockPosition::from(start);

        while (distance_next.min_element() * forward).length_squared()
            < start.distance_squared(goal)
        {
            let next = distance_next.min_element();
            if distance_next.x == next {
                block_position.x += step.x;
                distance_next.x += distance_increment.x;
            } else {
                block_position.z += step.y;
                distance_next.y += distance_increment.y;
            }

            if block_position == BlockPosition::from(goal) {
                self.path.push(goal);
                return;
            }

            let above_cost = self.get_movement_cost(world_map, block_position + IVec3::Y);
            let cost = self.get_movement_cost(world_map, block_position);
            let below_cost = self.get_movement_cost(world_map, block_position - IVec3::Y);
            let second_below_cost =
                self.get_movement_cost(world_map, block_position - IVec3::Y * 2);

            if above_cost.is_none() {
                // If there's a block at head height, fail
                return;
            }

            if cost.is_none() {
                // jump up one block
                block_position.y += 1;
                continue;
            }

            if below_cost.is_none() {
                // Move forward
                continue;
            } else {
                // Move down one block
                block_position.y -= 1;

                if second_below_cost.is_some() {
                    // fall down
                    block_position.y -= 1;
                }
            }
        }
    }

    pub fn next_node(&mut self, current_postition: DVec3) -> Option<DVec3> {
        while let Some(next_position) = self.path.last() {
            if next_position.xz().distance_squared(current_postition.xz()) >= 0.25 {
                return Some(*next_position);
            }

            self.previous_node = self.path.pop();
        }
        None
    }

    pub fn previous_node(&self) -> Option<DVec3> {
        self.previous_node
    }

    fn get_movement_cost(&mut self, world_map: &WorldMap, position: BlockPosition) -> Option<f32> {
        if let Some(cached) = self.movement_cost_cache.get(&position) {
            return *cached;
        }

        let compute_cost = |position: BlockPosition| -> Option<f32> {
            if let Some(block_id) = world_map.get_block(position) {
                let block_config = Blocks::get().get_config(&block_id);
                if let Some(drag) = block_config.drag() {
                    return Some(drag.max_element() as f32);
                }
            }

            return None;
        };

        let mut movement_cost = Some(0.0);

        for x in 0..self.width {
            for z in 0..self.width {
                for y in 0..self.height {
                    let position = position + IVec3::new(x, y, z);
                    if let Some(mc) = compute_cost(position) {
                        movement_cost
                            .as_mut()
                            .map(|movement_cost| *movement_cost += mc);
                    } else {
                        // Even if we could exit early here, we continue iterating to add all
                        // non-traversable blocks to the cache so we'll exit earlier on consecutive
                        // cost lookups.
                        self.movement_cost_cache.insert(position, None);
                        movement_cost = None;
                    }
                }
            }
        }

        self.movement_cost_cache.insert(position, movement_cost);

        return movement_cost;
    }

    fn heuristic_cost(&self, position: BlockPosition) -> f32 {
        position.distance_squared(*self.goal) as f32
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
        &mut self,
        position: &BlockPosition,
        world_map: &WorldMap,
    ) -> SmallVec<[PotentialSuccessor; 4]> {
        let mut potential_successors = SmallVec::default();
        for offset in [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z].iter() {
            let offset_position = *position + *offset;

            if let Some(mut movement_cost) = self.get_movement_cost(world_map, offset_position) {
                // If it can move horizontally, check if and how far it will fall
                // Hardcoded to only fall a maximum of two blocks
                for steps in 1..=2 {
                    let below_position = offset_position - IVec3::new(0, steps, 0);
                    if let Some(below_cost) = self.get_movement_cost(world_map, below_position) {
                        movement_cost += below_cost;
                    } else {
                        let position = offset_position - IVec3::new(0, steps - 1, 0);
                        potential_successors.push(PotentialSuccessor {
                            position,
                            movement_cost,
                            heuristic_cost: self.heuristic_cost(position),
                        });
                        break;
                    }
                }
            } else if self.jump_height > 0 {
                // Hit a wall, try to jump up
                for j in 1..=self.jump_height as i32 {
                    let jump_position = offset_position + IVec3::new(0, j, 0);
                    let above_position = *position + IVec3::new(0, j, 0);
                    if let Some(movement_cost) = self.get_movement_cost(world_map, jump_position)
                        && self.get_movement_cost(world_map, above_position).is_some()
                    {
                        potential_successors.push(PotentialSuccessor {
                            position: jump_position,
                            movement_cost: movement_cost + j as f32,
                            heuristic_cost: self.heuristic_cost(jump_position),
                        });
                    }
                }
            }
        }

        return potential_successors;
    }

    fn set_path(
        &mut self,
        mut index: usize,
        node_map: &IndexMap<BlockPosition, PathNode>,
        accurate_goal: Option<DVec3>,
    ) {
        let xz_offset = DVec3::new(self.width as f64 / 2.0, 0.0, self.width as f64 / 2.0);
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
        // Same, but since the npc will already be at the start position, it can be removed.
        self.path.pop();
    }
}

// The current best cost of the path to a node
struct PathNode {
    parent_index: usize,
    cost: f32,
}

struct PotentialSuccessor {
    position: BlockPosition,
    movement_cost: f32,
    heuristic_cost: f32,
}

// A possible new best path node
struct Successor {
    node_index: usize,
    // The cumulative move cost of the node
    movement_cost: f32,
    // The node's distance from the goal
    heuristic_cost: f32,
}

impl Successor {
    fn cost(&self) -> f32 {
        self.movement_cost + self.heuristic_cost
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
        self.movement_cost.eq(&other.movement_cost) && self.heuristic_cost.eq(&other.heuristic_cost)
    }
}

impl Eq for Successor {}

impl PartialOrd for Successor {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
