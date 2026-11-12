use std::ops::{Index, IndexMut};

use fmc::{
    bevy::platform::collections::{HashMap, HashSet},
    blocks::{BlockFace, BlockId, BlockPosition, BlockRotation, BlockState, Blocks},
    prelude::*,
    world::{BlockUpdate, ChangedBlockEvent},
};

// NOTE: The way this has been done seems to be the wrong approach. I've ended up just embracing the
// complexity in order to make a semi-functioning system, but another approach is sorely needed. The
// complexity comes from trying to compress what is 10k(10 levels for each corner) different water
// states into 1k states. Each state needs its own block, so the compression is done to reduce the
// amount block assets needed for each liquid that is added. It all seems like a very poor decision,
// 1k is already too much to tolerate and it just makes it ugly, terrible compromise.
//
// Possible solutions:
// 1. Hardcode a "liquid" type of block in the client.
//    The water level alone is enough to determine the correct shape so you'd only need a single
//    block asset.
//    Feels like a breach of the client/server distinction, but if the other two options are the
//    competition this seems like the best way out.
//
// 2. Letting blocks define their vertices/uvs/textures etc through expressions in the assets. Which
//    would mean some sort of expression evaluator and the performance implications of using
//    something like that in the mesher. All too complicated to get into for now.
//
// 3. Meshing done through wasm?

pub(super) struct WaterPlugin;
impl Plugin for WaterPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WaterUpdateTimer(Timer::new(
            std::time::Duration::from_millis(500),
            TimerMode::Repeating,
        )))
        .add_systems(Startup, setup)
        .add_systems(Update, update_water);
    }
}

#[derive(Debug, Clone, Copy)]
enum Corner {
    Left,
    Right,
    FarRight,
    FarLeft,
}

impl Corner {
    fn translate(self, offset: IVec3) -> Self {
        let mut corner = self;

        if offset.x != 0 {
            corner = match corner {
                Self::Left => Self::Right,
                Self::Right => Self::Left,
                Self::FarLeft => Self::FarRight,
                Self::FarRight => Self::FarLeft,
            };
        }

        if offset.z != 0 {
            corner = match corner {
                Self::Left => Self::FarLeft,
                Self::FarLeft => Self::Left,
                Self::Right => Self::FarRight,
                Self::FarRight => Self::Right,
            };
        }

        corner
    }

    fn neighbourhood(self) -> [IVec3; 4] {
        match self {
            Self::Left => [IVec3::ZERO, IVec3::NEG_X, IVec3::Z, IVec3::new(-1, 0, 1)],
            Self::Right => [IVec3::ZERO, IVec3::X, IVec3::Z, IVec3::new(1, 0, 1)],
            Self::FarRight => [IVec3::ZERO, IVec3::X, IVec3::NEG_Z, IVec3::new(1, 0, -1)],
            Self::FarLeft => [
                IVec3::ZERO,
                IVec3::NEG_X,
                IVec3::NEG_Z,
                IVec3::new(-1, 0, -1),
            ],
        }
    }

    fn adjacent_corners(self) -> [Corner; 2] {
        match self {
            Self::Left => [Self::Right, Self::FarLeft],
            Self::Right => [Self::Left, Self::FarRight],
            Self::FarRight => [Self::FarLeft, Self::Right],
            Self::FarLeft => [Self::FarRight, Self::Left],
        }
    }

    fn all() -> [Corner; 4] {
        [
            Corner::Left,
            Corner::Right,
            Corner::FarRight,
            Corner::FarLeft,
        ]
    }
}

#[derive(Clone, Copy, Debug)]
struct WaterBlock {
    level: u8,
    corners: [CornerLevels; 4],
    source: bool,
}

#[derive(Clone, Copy, Debug)]
struct CornerLevels([Option<u8>; 4]);

impl CornerLevels {
    const EMPTY: Self = Self([Some(0); 4]);

    fn new(level: u8) -> Self {
        Self([Some(level); 4])
    }

    fn levels(&self) -> impl Iterator<Item = u8> {
        self.0.into_iter().flatten()
    }

    fn height(&self) -> u8 {
        self.levels().max().unwrap_or(0)
    }

    fn is_isolated(&self, corner: Corner) -> bool {
        let enclosed = corner
            .adjacent_corners()
            .into_iter()
            .all(|adjacent| self[adjacent].is_none());

        let height = self.height();
        let connected = self
            .levels()
            .filter(|&level| level != 0 && level + 2 >= height)
            .count();

        !enclosed && connected == 1
    }

    fn cap(&mut self, max: u8) {
        self.0 = self.0.map(|level| level.map(|level| level.min(max)));
    }
}

impl Index<Corner> for CornerLevels {
    type Output = Option<u8>;

    fn index(&self, corner: Corner) -> &Self::Output {
        &self.0[corner as usize]
    }
}

impl IndexMut<Corner> for CornerLevels {
    fn index_mut(&mut self, corner: Corner) -> &mut Self::Output {
        &mut self.0[corner as usize]
    }
}

impl PartialEq for WaterBlock {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key() && self.level == other.level && self.source == other.source
    }
}

impl Eq for WaterBlock {}

impl WaterBlock {
    const EMPTY: WaterBlock = WaterBlock {
        level: 0,
        corners: [CornerLevels::EMPTY; 4],
        source: false,
    };

    // Uses the block's filename to derive the water level as water_<back_right>_<back_left>_<front_right>_<front_left>
    fn parse(block_name: &str) -> Option<([u8; 4], bool)> {
        let mut name_parts = block_name.split('_');

        if name_parts.next()? != "water" {
            return None;
        }

        let mut levels = [0; 4];
        for level in levels.iter_mut() {
            let Some(name_part) = name_parts.next() else {
                panic!("Malformed water block name '{block_name}', expected four corner levels");
            };

            let number: u8 = match name_part.parse() {
                Ok(number) => number,
                Err(_) => panic!(
                    "Malformed water block name '{block_name}', expected a number, found '{name_part}'"
                ),
            };

            if number > MAX_HEIGHT {
                panic!(
                    "Malformed water block name '{block_name}', the corner level {number} is larger \
                    than the maximum of {MAX_HEIGHT}"
                );
            }

            *level = number;
        }

        let flowing = match name_parts.next() {
            None => false,
            Some("straight") => true,
            Some(name_part) => panic!(
                "Malformed water block name '{block_name}', expected 'straight', found '{name_part}'"
            ),
        };

        if name_parts.next().is_some() {
            panic!(
                "Malformed water block name '{block_name}', expected only four corner levels and \
                an optional '_straight' suffix"
            );
        }

        if flowing && levels.iter().any(|&level| level != levels[0]) {
            panic!("Malformed water block name '{block_name}', only flat water can be 'straight'");
        }

        let [back_right, back_left, front_right, front_left] = levels;
        Some(([front_left, front_right, back_right, back_left], flowing))
    }

    fn is_flat(&self) -> bool {
        let heights = self.corners.map(|levels| levels.height());
        heights.iter().all(|&height| height == heights[0])
    }

    fn get_corner(&self, corner: Corner) -> u8 {
        let levels = self.corners[corner as usize];
        if levels.is_isolated(corner) && !self.is_flat() {
            levels.height().saturating_sub(4)
        } else {
            levels.height()
        }
    }

    fn key(&self) -> [u8; 4] {
        let still = Corner::all().map(|corner| self.get_corner(corner) * 2);

        let low = self.corners.map(|corner| {
            let max = corner.height();
            // This determines if the levels around a corner would have an average of less than
            // max-0.5. It's guaranteed if there are two levels that have a level lower than the max.
            corner
                .levels()
                .filter(|&level| level < max && level != 0)
                .count()
                > 2
        });

        let flowing = low.iter().filter(|&&low| low).count() == 2 && low[0] != low[2];

        if self.is_flat() && flowing {
            std::array::from_fn(|index| still[index] - low[index] as u8)
        } else {
            still
        }
    }

    fn balance(&mut self) {
        if self.level == 0 {
            *self = WaterBlock::EMPTY;
            return;
        }

        let unbalanced = *self;
        for corner in Corner::all() {
            let max = Corner::all()
                .into_iter()
                .filter(|&other| other as u8 != corner as u8)
                .map(|other| unbalanced.get_corner(other))
                .max()
                .unwrap();
            let current = self.get_corner(corner);
            if current != 0 && max.saturating_sub(current) <= MAX_DROP {
                continue;
            }

            self.corners[corner as usize] = CornerLevels::new(max.saturating_sub(MAX_DROP).max(1));
        }

        // [1;4] corners are often leftover from decay and is unwanted from spreading, removed for
        // aesthetics so it doesn't leave disconnected puddles on the ground.
        if Corner::all()
            .into_iter()
            .all(|corner| self.get_corner(corner) <= 1)
        {
            *self = WaterBlock::EMPTY;
        }
    }
}

const MAX_HEIGHT: u8 = 10;
const MAX_LEVEL: u8 = 8;
const MAX_DROP: u8 = 5;

fn setup(mut commands: Commands, blocks: Res<Blocks>) {
    // Since generated water is "source" water, we'd have to store block state for each generated water
    // block. To avoid this we store two unique blocks that translate to a water level, but not the
    // other way around, so that ocean water is never re-derived, when it's been replaced it's gone
    // for good.
    let ocean_top = blocks.get_id("ocean_top");
    let ocean_bottom = blocks.get_id("ocean_bottom");

    let mut water = Water {
        water_to_block: HashMap::default(),
        block_to_water: HashMap::default(),
        natural_sources: [ocean_top, ocean_bottom],
    };

    water.block_to_water.insert((ocean_top, None), [8 * 2; 4]);
    water
        .block_to_water
        .insert((ocean_bottom, None), [10 * 2; 4]);

    for (block_name, block_id) in blocks.ids() {
        if let Some((corners, flowing)) = WaterBlock::parse(block_name) {
            water.add(*block_id, corners, flowing);
        }
    }

    let air = blocks.get_id("air");
    water
        .block_to_water
        .insert((air, None), WaterBlock::EMPTY.key());
    water
        .water_to_block
        .insert(WaterBlock::EMPTY.key(), (air, None));

    commands.insert_resource(water);
}

#[derive(Resource)]
pub struct Water {
    water_to_block: HashMap<[u8; 4], (BlockId, Option<BlockRotation>)>,
    block_to_water: HashMap<(BlockId, Option<BlockRotation>), [u8; 4]>,
    natural_sources: [BlockId; 2],
}

impl Water {
    const SOURCE_BIT: u16 = 0b1000_0000_0000_0000;
    const LEVEL_SHIFT: u16 = 11;
    const LEVEL_MASK: u16 = 0b0111_1000_0000_0000;

    fn add(&mut self, block_id: BlockId, corners: [u8; 4], flowing: bool) {
        // Registered under all of its rotations so that a shape can be looked up without
        // knowing which rotation it was given a block for. Shapes that rotate into themselves
        // are claimed by the first rotation that reaches them, the order matters only for
        // those.
        for rotation in [
            BlockRotation::Front,
            BlockRotation::Left,
            BlockRotation::Back,
            BlockRotation::Right,
        ] {
            let mut key = [0; 4];
            for (index, corner) in corners.into_iter().enumerate() {
                key[(index + rotation as usize) % 4] = corner * 2;
            }

            if flowing {
                for index in [rotation as usize, (rotation as usize + 1) % 4] {
                    key[index] -= 1;
                }
            }

            self.block_to_water.insert((block_id, Some(rotation)), key);

            // Some water states are symmetric, the first one claims the spot
            self.water_to_block
                .entry(key)
                .or_insert((block_id, Some(rotation)));
        }
    }

    fn to_block(&self, water_block: &WaterBlock) -> Option<(BlockId, Option<BlockState>)> {
        let (block_id, rotation) = *self.water_to_block.get(&water_block.key())?;

        let block_state = rotation.map(|rotation| {
            let mut state = BlockState::new().with_rotation(rotation);
            state.0 |= (water_block.level as u16) << Self::LEVEL_SHIFT;
            if water_block.source {
                state.0 |= Self::SOURCE_BIT;
            }
            state
        });

        Some((block_id, block_state))
    }

    fn from_block(&self, block: (BlockId, Option<BlockState>)) -> Option<WaterBlock> {
        let block_rotation = block.1.and_then(|block_state| block_state.rotation());
        let key = *self.block_to_water.get(&(block.0, block_rotation))?;
        let state = block.1.map_or(0, |state| state.as_u16());

        let source = state & Self::SOURCE_BIT != 0 || self.natural_sources.contains(&block.0);
        let level = if source {
            8
        } else {
            ((state & Self::LEVEL_MASK) >> Self::LEVEL_SHIFT) as u8
        };

        // The corners must map back to the same key. Flat water that is flowing has odd keys in
        // two corners, in order to make the corner map back to an odd key we give one of the
        // elements the max value and the other three the value 1 below. If the key is even, it
        // will just be the max in all four fields.
        let mut corners = key.map(|key| CornerLevels::new(key / 2));
        for corner in Corner::all() {
            // The highest value must be set in the corner that actually owns it.
            corners[corner as usize][corner] = Some(key[corner as usize].div_ceil(2));
        }

        Some(WaterBlock {
            level,
            corners,
            source,
        })
    }

    pub fn is_source(&self, block_id: BlockId, block_state: Option<BlockState>) -> bool {
        self.from_block((block_id, block_state))
            .is_some_and(|water_block| water_block.source)
    }

    pub fn source_block(&self) -> (BlockId, Option<BlockState>) {
        self.to_block(&WaterBlock {
            level: MAX_LEVEL,
            corners: [CornerLevels::new(MAX_LEVEL); 4],
            source: true,
        })
        .unwrap()
    }
}

#[derive(Resource, DerefMut, Deref)]
struct WaterUpdateTimer(Timer);

#[derive(Clone, Copy, Default)]
struct Neighbourhood {
    previous: Option<WaterBlock>,
    current: [Option<WaterBlock>; 27],
}

impl Index<IVec3> for Neighbourhood {
    type Output = Option<WaterBlock>;

    fn index(&self, offset: IVec3) -> &Self::Output {
        assert!(
            offset.abs().max_element() <= 1,
            "Offset {offset} is outside the neighbourhood"
        );
        &self.current[((offset.x + 1) * 9 + (offset.y + 1) * 3 + (offset.z + 1)) as usize]
    }
}

impl IndexMut<IVec3> for Neighbourhood {
    fn index_mut(&mut self, offset: IVec3) -> &mut Self::Output {
        assert!(
            offset.abs().max_element() <= 1,
            "Offset {offset} is outside the neighbourhood"
        );
        &mut self.current[((offset.x + 1) * 9 + (offset.y + 1) * 3 + (offset.z + 1)) as usize]
    }
}

impl Neighbourhood {
    fn new(event: &ChangedBlockEvent, water: &Water) -> Option<Self> {
        let mut neighbourhood = Self::default();
        let mut any_water = false;

        let mut read_water_block = |offset: IVec3| -> Option<WaterBlock> {
            let water_block = event[offset].and_then(|block| water.from_block(block));

            any_water |= water_block.is_some_and(|block| block != WaterBlock::EMPTY);
            water_block
        };

        neighbourhood.previous = water.from_block(event.from);

        neighbourhood[IVec3::ZERO] = read_water_block(IVec3::ZERO);

        for first in BlockFace::iter() {
            let offset = first.offset();
            let Some(water_block) = read_water_block(offset) else {
                continue;
            };
            neighbourhood[offset] = Some(water_block);

            for second in BlockFace::iter() {
                if second == first || second == first.opposite() {
                    continue;
                }

                let offset = offset + second.offset();
                if neighbourhood[offset].is_some() {
                    continue;
                }

                let Some(water_block) = read_water_block(offset) else {
                    continue;
                };
                neighbourhood[offset] = Some(water_block);

                for third in BlockFace::iter() {
                    if third == first
                        || third == first.opposite()
                        || third == second
                        || third == second.opposite()
                    {
                        continue;
                    }

                    let offset = offset + third.offset();
                    if neighbourhood[offset].is_none() {
                        neighbourhood[offset] = read_water_block(offset);
                    }
                }
            }
        }

        any_water.then_some(neighbourhood)
    }

    fn with_pending(
        &mut self,
        position: BlockPosition,
        pending_updates: &HashMap<BlockPosition, WaterBlock>,
    ) {
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1 {
                    let offset = IVec3::new(x, y, z);
                    if self[offset].is_none() {
                        continue;
                    }

                    if let Some(pending) = pending_updates.get(&(position + offset)) {
                        self[offset] = Some(*pending);
                    }
                }
            }
        }
    }

    fn center(&self) -> Option<WaterBlock> {
        return self[IVec3::ZERO];
    }

    fn update_levels(&self) -> Self {
        let mut updated = *self;

        if let Some(center) = updated[IVec3::ZERO].as_mut()
            && !center.source
        {
            center.level = if self[IVec3::Y].is_some_and(|above| above != WaterBlock::EMPTY) {
                MAX_LEVEL
            } else {
                [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z]
                    .into_iter()
                    .filter(|&offset| {
                        // Can only be taken from if either this block has a solid beneath it or the
                        // adjacent one does.
                        self[IVec3::NEG_Y].is_none() || self[offset + IVec3::NEG_Y].is_none()
                    })
                    .filter_map(|offset| self[offset])
                    .map(|water_block| water_block.level)
                    .max()
                    .unwrap_or(0)
                    .saturating_sub(1)
            };
        }

        let level = updated.center().map_or(0, |center| center.level);
        let was_water = self
            .center()
            .is_some_and(|center| center != WaterBlock::EMPTY);

        if level >= 2 && was_water && self[IVec3::NEG_Y].is_none() {
            for offset in [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z] {
                if let Some(water_block) = updated[offset].as_mut()
                    && water_block.level < level - 1
                {
                    water_block.level = level - 1;
                }
            }
        }

        if level > 0
            && let Some(below) = updated[IVec3::NEG_Y].as_mut()
        {
            below.level = MAX_LEVEL;
        }

        updated
    }

    fn update_corners(&mut self) {
        for x in -1..=1 {
            for z in -1..=1 {
                let position = IVec3::new(x, 0, z);
                let level = self[position].map(|water_block| water_block.level);

                for corner in Corner::all() {
                    for offset in corner.neighbourhood() {
                        let neighbour = position + offset;
                        if neighbour.abs().max_element() > 1 {
                            continue;
                        }

                        let Some(water_block) = self[neighbour].as_mut() else {
                            continue;
                        };

                        if water_block.level == 0 {
                            continue;
                        }

                        water_block.corners[corner.translate(offset) as usize][corner] = level;
                    }
                }
            }
        }

        for y in [0, -1] {
            for x in -1..=1 {
                for z in -1..=1 {
                    let offset = IVec3::new(x, y, z);
                    if !self[offset].is_some_and(|water_block| water_block.level > 0) {
                        continue;
                    }

                    for corner in Corner::all() {
                        let mut water_above = false;
                        let mut outside_neighbourhood = false;

                        for corner_offset in corner.neighbourhood() {
                            let above = offset + corner_offset + IVec3::Y;
                            if above.abs().max_element() > 1 {
                                outside_neighbourhood = true;
                            } else if self[above].is_some_and(|above| above.level > 0) {
                                water_above = true;
                            }
                        }

                        let levels = &mut self[offset].as_mut().unwrap().corners[corner as usize];

                        if water_above {
                            *levels = CornerLevels::new(MAX_HEIGHT);
                        } else if !outside_neighbourhood {
                            levels.cap(8);
                        }
                    }
                }
            }
        }
    }
}

#[derive(Default)]
struct Queue {
    // Events are ordered from highest to lowest level so that those with the highest level can be
    // processed first.
    bins: [Vec<BlockPosition>; 11],
    cache: HashMap<BlockPosition, Neighbourhood>,
    // Only positions that are changed into non-water or are included in this hashset during the
    // previous update will be processed this tick. This is to avoid water spreading diagonally.
    tickable: HashSet<BlockPosition>,
}

impl Queue {
    fn insert(&mut self, position: BlockPosition, neighbourhood: Neighbourhood) {
        // We only recompute the levels of positions that were queued the last tick, or if it's a
        // block that has been placed/removed
        if neighbourhood
            .center()
            .is_some_and(|block| block != WaterBlock::EMPTY && !block.source)
            && !self.tickable.contains(&position)
        {
            return;
        }

        let level = neighbourhood
            .center()
            .or(neighbourhood.previous)
            .unwrap_or(WaterBlock::EMPTY)
            .level as usize;

        if self.cache.insert(position, neighbourhood).is_none() {
            // Avoids multiple entries in bins
            self.bins[level].push(position);
        }
    }

    fn drain(&mut self) -> impl Iterator<Item = (BlockPosition, Neighbourhood)> {
        self.tickable.clear();

        self.bins
            .iter_mut()
            .rev()
            .flat_map(|bin| bin.drain(..))
            .map(|position| {
                self.tickable.insert(position);
                for face in BlockFace::iter() {
                    self.tickable.insert(position + face.offset());
                }

                (position, self.cache.remove(&position).unwrap())
            })
    }
}

fn update_water(
    water: Res<Water>,
    time: Res<Time>,
    mut update_timer: ResMut<WaterUpdateTimer>,
    mut changed_blocks: MessageReader<ChangedBlockEvent>,
    mut block_updates: MessageWriter<BlockUpdate>,
    mut pending_updates: Local<HashMap<BlockPosition, WaterBlock>>,
    mut queue: Local<Queue>,
) {
    update_timer.tick(time.delta());

    for event in changed_blocks.read() {
        let Some(neighbourhood) = Neighbourhood::new(event, &water) else {
            continue;
        };

        // If water spreads into water from another source the corners will most likely not match. We
        // immediately fix this. The pending_updates will be drained and we will receive a new
        // ChangedBlockEvent that replaces this one, but has matching corners.
        for corner in Corner::all() {
            let Some(max) = corner
                .neighbourhood()
                .into_iter()
                .filter_map(|offset| {
                    let neighbour = neighbourhood[offset]?;
                    Some(neighbour.get_corner(corner.translate(offset)))
                })
                .max()
            else {
                continue;
            };

            for offset in corner.neighbourhood() {
                let Some(water_block) = neighbourhood[offset] else {
                    continue;
                };

                if water_block == WaterBlock::EMPTY {
                    continue;
                }

                let corner = corner.translate(offset);
                if water_block.get_corner(corner) < max {
                    pending_updates
                        .entry(event.position + offset)
                        .or_insert(water_block)
                        .corners[corner as usize] = CornerLevels::new(max);
                }
            }
        }

        queue.insert(event.position, neighbourhood);
    }

    if update_timer.just_finished() {
        for (position, mut neighbourhood) in queue.drain() {
            neighbourhood.with_pending(position, &pending_updates);
            // let updated = neighbourhood.update();
            let mut updated = neighbourhood.update_levels();
            updated.update_corners();

            for x in -1..=1 {
                for y in -1..=1 {
                    for z in -1..=1 {
                        let offset = IVec3::new(x, y, z);
                        let Some(water_block) = updated[offset] else {
                            continue;
                        };

                        let mut balanced = water_block;
                        balanced.balance();

                        if neighbourhood[offset] != Some(balanced) {
                            pending_updates.insert(position + offset, water_block);
                        }
                    }
                }
            }
        }
    }

    for (position, mut water_block) in pending_updates.drain() {
        water_block.balance();

        let Some((block_id, block_state)) = water.to_block(&water_block) else {
            error!(
                "No water block covers the corners {:?}",
                water_block.corners
            );
            continue;
        };

        block_updates.write(BlockUpdate::Replace {
            position,
            block_id,
            block_state,
            block_data: None,
        });
    }
}
