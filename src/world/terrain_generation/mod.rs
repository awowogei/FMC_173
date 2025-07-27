use fmc::{
    blocks::Blocks,
    noise::{Frequency, Noise},
    // noise::Noise,
    prelude::*,
    utils::Rng,
    world::{
        chunk::{Chunk, ChunkPosition},
        Surface, TerrainGenerator,
    },
};

use rand::SeedableRng;

mod biomes;
mod blueprints;

pub struct Earth {
    biomes: biomes::Biomes,
    continents: Noise,
    terrain_height: Noise,
    terrain_shape: Noise,
    caves: Noise,
    seed: u64,
}

impl TerrainGenerator for Earth {
    fn generate_chunk(&self, chunk_position: ChunkPosition) -> Chunk {
        let mut chunk = Chunk::default();

        let air = Blocks::get().get_id("air");
        const MAX_HEIGHT: i32 = 120;
        if MAX_HEIGHT < chunk_position.y {
            // Don't waste time generating if it is guaranteed to be air.
            chunk.make_uniform(air);
        } else {
            self.generate_terrain(chunk_position, &mut chunk);

            // TODO: Might make sense to test against water too.
            //
            // Test for air chunk uniformity early so we can break and elide the other generation
            // functions. This makes it so all other chunks that are uniform with another type of
            // block get stored as full size chunks. They are assumed to be very rare.
            let mut uniform = true;
            for block in chunk.blocks.iter() {
                if *block != air {
                    uniform = false;
                    break;
                }
            }

            if uniform {
                chunk.make_uniform(air);
                return chunk;
            }

            self.generate_features(chunk_position, &mut chunk);
        }

        return chunk;
    }
}

// We generate a few blocks above the chunk because we need the information for placing surface
// blocks.
const CHUNK_HEIGHT: usize = Chunk::SIZE + TERRAIN_HEIGHT_FACTOR;
const CONTINTENT_MAX: f32 = 10.0;
const CONTINTENT_MIN: f32 = -10.0;
// How many blocks there are per interpolation segment
const TERRAIN_WIDTH_FACTOR: usize = 4;
const TERRAIN_HEIGHT_FACTOR: usize = 8;
const CAVES_WIDTH_FACTOR: usize = 4;
const CAVES_HEIGHT_FACTOR: usize = 4;
// +1 because you need an extra point to generate a segment. i.e the segment 0..1 needs two points,
// 0..1..3 is two segments etc
const TERRAIN_WIDTH: usize = Chunk::SIZE / TERRAIN_WIDTH_FACTOR + 1;
const TERRAIN_HEIGHT: usize = CHUNK_HEIGHT / TERRAIN_HEIGHT_FACTOR + 1;
const CAVES_WIDTH: usize = Chunk::SIZE / CAVES_WIDTH_FACTOR + 1;
const CAVES_HEIGHT: usize = CHUNK_HEIGHT / CAVES_HEIGHT_FACTOR + 1;

impl Earth {
    pub fn new(seed: u64, blocks: &Blocks) -> Self {
        let mut rng = Rng::new(seed);

        let freq = 1.0 / 2f32.powi(9) * 3.0;
        // let freq = 0.00305;
        let continents = Noise::perlin(Frequency {
            x: freq,
            y: 0.0,
            z: freq,
        })
        .seed(rng.next_u32())
        .fbm(4, 0.5, 2.0)
        .abs()
        // Only really interested in the high and low values, in-between should quickly transition.
        .mul(Noise::constant(120.0))
        // sea
        .add(Noise::constant(-12.0))
        .clamp(CONTINTENT_MIN, CONTINTENT_MAX);

        //let freq = 1.0 / 2.0f32.powi(5);
        let freq = 0.002189;
        let terrain_height = continents
            .clone()
            .range(
                -2.0,
                2.0,
                Noise::constant(0.0),
                // Noise::constant(1.5),
                Noise::perlin(freq)
                    .seed(rng.next_u32())
                    .fbm(10, 0.5, 2.0)
                    .mul(Noise::constant(2.0))
                    .add(Noise::constant(1.0)),
            )
            .add(Noise::constant(0.5))
            .clamp(0.5, 1.5);

        let freq = 0.0313;
        let freq = Frequency {
            x: freq,
            y: freq * 1.5,
            z: freq,
        };
        let high = Noise::perlin(freq).seed(rng.next_u32()).fbm(6, 0.5, 2.0);
        let low = Noise::perlin(freq).seed(rng.next_u32()).fbm(6, 0.5, 2.0);

        // NOTE: Because of interpolation the noise is stretched. 4x horizontally and 8x
        // vertically.
        //
        // High and low are switched between to create sudden changes in terrain elevation.
        //let freq = 0.03379;
        // let freq = 1.0 / 2.0f32.powi(4);
        let terrain_shape = Noise::simplex(Frequency {
            x: freq.x * 1.5,
            y: freq.y * 1.5 * 0.5,
            z: freq.z * 1.5,
        })
        .seed(rng.next_u32())
        .fbm(8, 0.5, 2.0)
        .range(0.00, 0.02, low, high);

        // Visualization: https://www.shadertoy.com/view/stccDB
        let freq = 0.02;
        let y_freq = freq;
        let octaves = 5;
        let cave_main = Noise::perlin(Frequency {
            x: freq,
            y: y_freq,
            z: freq,
        })
        .seed(rng.next_u32())
        .fbm(octaves, 0.5, 2.0)
        .square();
        let cave_main_2 = Noise::perlin(Frequency {
            x: freq,
            y: y_freq,
            z: freq,
        })
        .seed(rng.next_u32())
        .fbm(octaves, 0.5, 2.0)
        .square();
        let caves = cave_main.add(cave_main_2);

        let cave_main_3 = Noise::perlin(Frequency {
            x: freq,
            y: y_freq,
            z: freq,
        })
        .seed(rng.next_u32())
        .fbm(octaves, 0.5, 2.0)
        .square();
        let cave_main_4 = Noise::perlin(Frequency {
            x: freq,
            y: y_freq,
            z: freq,
        })
        .seed(rng.next_u32())
        .fbm(octaves, 0.5, 2.0)
        .square();
        let caves = cave_main_3.add(cave_main_4).min(caves);

        Self {
            biomes: biomes::Biomes::load(blocks),
            continents,
            terrain_height,
            terrain_shape,
            caves,
            seed,
        }
    }

    fn generate_terrain(&self, chunk_position: ChunkPosition, chunk: &mut Chunk) {
        let chunk_x = (chunk_position.x / (TERRAIN_WIDTH_FACTOR as i32)) as f32;
        let chunk_y = (chunk_position.y / (TERRAIN_HEIGHT_FACTOR as i32)) as f32;
        let chunk_z = (chunk_position.z / (TERRAIN_WIDTH_FACTOR as i32)) as f32;
        let (mut terrain, _, _) = self.terrain_shape.generate_3d(
            chunk_x,
            chunk_y,
            chunk_z,
            TERRAIN_WIDTH,
            TERRAIN_HEIGHT,
            TERRAIN_WIDTH,
        );

        let (continent_height, _, _) =
            self.continents
                .generate_2d(chunk_x, chunk_z, TERRAIN_WIDTH, TERRAIN_WIDTH);

        let (terrain_height, _, _) =
            self.terrain_height
                .generate_2d(chunk_x, chunk_z, TERRAIN_WIDTH, TERRAIN_WIDTH);

        for x in 0..TERRAIN_WIDTH {
            for z in 0..TERRAIN_WIDTH {
                let index = x * TERRAIN_WIDTH + z;
                let continent_height = continent_height[index];
                let terrain_height = terrain_height[index];
                for y in 0..TERRAIN_HEIGHT {
                    // Amount the density should be decreased by per block above the base height.
                    const DECREMENT: f32 = 0.015;
                    let mut compression = ((chunk_position.y + (y * TERRAIN_HEIGHT_FACTOR) as i32)
                        as f32
                        - continent_height)
                        * DECREMENT
                        / terrain_height;
                    if compression < 0.0 {
                        // Below surface, extra compression
                        compression *= 4.0;
                    }
                    let index = x * (TERRAIN_WIDTH * TERRAIN_HEIGHT) + z * TERRAIN_HEIGHT + y;

                    // Decrease density if above base height, increase if below
                    terrain[index] -= compression;
                }
            }
        }

        let mut terrain_shape = interpolate(&terrain);
        let continent_height = interpolate_continent_height(&continent_height);

        self.carve_caves(chunk_position, &continent_height, &mut terrain_shape);

        chunk.blocks = vec![0; Chunk::SIZE.pow(3)];

        let biome = self.biomes.get_biome();

        for x in 0..Chunk::SIZE {
            for z in 0..Chunk::SIZE {
                let mut layer = 0;

                let continent_height = continent_height[x * Chunk::SIZE + z];

                let mut liquid = false;

                // Find how deep we are from above chunk.
                for y in (Chunk::SIZE..CHUNK_HEIGHT).rev() {
                    let block_height = chunk_position.y + y as i32;
                    let block_index = x * (Chunk::SIZE * CHUNK_HEIGHT) + z * CHUNK_HEIGHT + y;
                    let density = terrain_shape[block_index];

                    if density <= 0.0 {
                        if block_height == 0 && continent_height < CONTINTENT_MAX {
                            liquid = true;
                        }
                        layer = 0;
                    } else {
                        liquid = false;
                        layer += 1;
                    }
                }

                for y in (0..Chunk::SIZE).rev() {
                    let block_height = chunk_position.y + y as i32;

                    let block_index = x * (Chunk::SIZE * CHUNK_HEIGHT) + z * CHUNK_HEIGHT + y;
                    let density = terrain_shape[block_index];

                    let block = if density <= 0.0 {
                        if block_height == 0 && continent_height < CONTINTENT_MAX {
                            layer = 1;
                            liquid = true;
                            biome.surface_liquid
                        } else if block_height < 0 && liquid {
                            biome.sub_surface_liquid
                        } else {
                            layer = 0;
                            biome.air
                        }
                    } else if layer > 3 {
                        layer += 1;
                        biome.bottom_layer_block
                    } else if block_height < 2
                        && block_height > CONTINTENT_MIN as i32 - 3
                        && continent_height <= 2.0
                    {
                        layer += 1;
                        biome.sand
                    } else {
                        let block = if layer < 1 && block_height >= 0 {
                            biome.top_layer_block
                        } else if layer < 3 && block_height > -1 {
                            biome.mid_layer_block
                        } else {
                            biome.bottom_layer_block
                        };
                        layer += 1;
                        block
                    };

                    chunk[[x, y, z]] = block;
                }
            }
        }
    }

    fn carve_caves(
        &self,
        chunk_position: ChunkPosition,
        continent_height: &Vec<f32>,
        terrain: &mut Vec<f32>,
    ) {
        let biome = self.biomes.get_biome();
        let chunk_x = (chunk_position.x / (CAVES_WIDTH_FACTOR as i32)) as f32;
        let chunk_y = (chunk_position.y / (CAVES_HEIGHT_FACTOR as i32)) as f32;
        let chunk_z = (chunk_position.z / (CAVES_WIDTH_FACTOR as i32)) as f32;

        let (caves, _, _) = self.caves.generate_3d(
            chunk_x,
            chunk_y,
            chunk_z,
            CAVES_WIDTH,
            CAVES_HEIGHT,
            CAVES_WIDTH,
        );
        let caves = interpolate_caves(&caves);

        // let air = Blocks::get().get_id("air");
        for x in 0..Chunk::SIZE {
            for z in 0..Chunk::SIZE {
                let continent_height_index = x * Chunk::SIZE + z;
                let continent_height = continent_height[continent_height_index];

                for y in 0..CHUNK_HEIGHT {
                    let index = x * Chunk::SIZE * CHUNK_HEIGHT + z * CHUNK_HEIGHT + y;
                    let mut cave_density = caves[index];
                    let height = chunk_position.y + y as i32;

                    let threshold = 0.001;
                    let decay_point = 0;
                    let density_offset = (height - decay_point).max(0) as f32 * threshold / 20.0;
                    cave_density += density_offset;

                    if cave_density < threshold
                        && (continent_height == CONTINTENT_MAX
                            || height < CONTINTENT_MIN as i32 - 10)
                    {
                        terrain[index] = -1.0;
                    }
                }
            }
        }

        // caves
        //     .into_iter()
        //     .zip(chunk.blocks.iter_mut())
        //     .enumerate()
        //     .for_each(|(i, (mut density, block))| {
        //         // TODO: Caves and water do not cooperate well. You carve the surface without
        //         // knowing there's water there and you get reverse moon pools underwater. Instead
        //         // we just push the caves underground, causing there to be no cave entraces at the
        //         // surface. There either needs to be a way to exclude caves from being generated
        //         // beneath water, or some way to intelligently fill carved out space that touches
        //         // water.
        //         const DECAY_POINT: i32 = -32;
        //         let y = chunk_position.y + (i & 0b1111) as i32;
        //         let density_offset = (y - DECAY_POINT).max(0) as f32 * 1.0 / 64.0;
        //         density += density_offset;
        //
        //         if (density / 2.0) < 0.001
        //             && *block != biome.surface_liquid
        //             && *block != biome.sub_surface_liquid
        //         {
        //             *block = air;
        //         }
        //     });
    }

    fn generate_features(&self, chunk_position: ChunkPosition, chunk: &mut Chunk) {
        let blocks = Blocks::get();
        let surface_blocks = [blocks.get_id("grass")];
        let surface = Surface::new(chunk, &surface_blocks, blocks.get_id("air"));

        // x position is left 32 bits and z position the right 32 bits. z must be converted to u32
        // first because it will just fill the left 32 bits with junk. World seed is used to change
        // which chunks are next to each other.
        let seed = ((chunk_position.x as u64) << 32 | chunk_position.z as u32 as u64)
            .overflowing_mul(self.seed)
            .0;
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);

        let biome = self.biomes.get_biome();

        for blueprint in biome.blueprints.iter() {
            blueprint.construct(chunk_position.into(), chunk, &surface, &mut rng);
        }
    }
}

// XXX: These interpolate functions are specific instead of generic 'interpolate_3d<HEIGHT, WIDTH,
// DEPTH>' etc because the compiler won't autovec them.
fn interpolate_continent_height(noise: &Vec<f32>) -> Vec<f32> {
    const WIDTH: usize = Chunk::SIZE / TERRAIN_WIDTH_FACTOR;
    const HEIGHT: usize = WIDTH;

    fn index(x: usize, z: usize) -> usize {
        return x * (HEIGHT + 1) + z;
    }

    let mut result = vec![0.0; Chunk::SIZE * Chunk::SIZE];

    for x_noise in 0..WIDTH {
        for z_noise in 0..HEIGHT {
            let mut back_left = noise[index(x_noise + 0, z_noise + 0)];
            let mut front_left = noise[index(x_noise + 0, z_noise + 1)];
            let mut back_right = noise[index(x_noise + 1, z_noise + 0)];
            let mut front_right = noise[index(x_noise + 1, z_noise + 1)];

            let back_increment = (back_right - back_left) * 0.25;
            let front_increment = (front_right - front_left) * 0.25;

            let mut back = back_left;
            let mut front = front_left;

            for x_index in 0..TERRAIN_WIDTH_FACTOR {
                let x = x_noise * TERRAIN_WIDTH_FACTOR + x_index;

                let middle_increment = (front - back) * 0.25;
                let mut density = back;

                for z_index in 0..TERRAIN_WIDTH_FACTOR {
                    let z = z_noise * WIDTH + z_index;
                    result[x * Chunk::SIZE + z] = density;
                    density += middle_increment;
                }

                back += back_increment;
                front += front_increment;
            }
        }
    }

    return result;
}

fn interpolate_caves(noise: &Vec<f32>) -> Vec<f32> {
    const WIDTH: usize = Chunk::SIZE / CAVES_WIDTH_FACTOR;
    const HEIGHT: usize = CHUNK_HEIGHT / CAVES_HEIGHT_FACTOR;
    const DEPTH: usize = WIDTH;
    const WIDTH_INCREMENT: f32 = 1.0 / CAVES_WIDTH_FACTOR as f32;
    const HEIGHT_INCREMENT: f32 = 1.0 / CAVES_HEIGHT_FACTOR as f32;

    fn index(x: usize, y: usize, z: usize) -> usize {
        return x * (DEPTH + 1) * (HEIGHT + 1) + z * (HEIGHT + 1) + y;
    }

    let mut result = vec![0.0; Chunk::SIZE * CHUNK_HEIGHT * Chunk::SIZE];

    for x_noise in 0..WIDTH {
        for z_noise in 0..DEPTH {
            for y_noise in 0..HEIGHT {
                let mut back_left = noise[index(x_noise + 0, y_noise + 0, z_noise + 0)];
                let mut front_left = noise[index(x_noise + 0, y_noise + 0, z_noise + 1)];
                let mut back_right = noise[index(x_noise + 1, y_noise + 0, z_noise + 0)];
                let mut front_right = noise[index(x_noise + 1, y_noise + 0, z_noise + 1)];
                let back_left_increment = (noise[index(x_noise + 0, y_noise + 1, z_noise + 0)]
                    - back_left)
                    * HEIGHT_INCREMENT;
                let front_left_increment = (noise[index(x_noise + 0, y_noise + 1, z_noise + 1)]
                    - front_left)
                    * HEIGHT_INCREMENT;
                let back_right_increment = (noise[index(x_noise + 1, y_noise + 1, z_noise + 0)]
                    - back_right)
                    * HEIGHT_INCREMENT;
                let front_right_increment = (noise[index(x_noise + 1, y_noise + 1, z_noise + 1)]
                    - front_right)
                    * HEIGHT_INCREMENT;

                for y_index in 0..CAVES_HEIGHT_FACTOR {
                    let y = y_noise * CAVES_HEIGHT_FACTOR + y_index;

                    let back_increment = (back_right - back_left) * WIDTH_INCREMENT;
                    let front_increment = (front_right - front_left) * WIDTH_INCREMENT;

                    let mut back = back_left;
                    let mut front = front_left;

                    for x_index in 0..CAVES_WIDTH_FACTOR {
                        let x = x_noise * WIDTH + x_index;

                        let bottom_increment = (front - back) * 0.25;
                        let mut density = back;

                        for z_index in 0..CAVES_WIDTH_FACTOR {
                            let z = z_noise * WIDTH + z_index;
                            result[x * Chunk::SIZE * CHUNK_HEIGHT + z * CHUNK_HEIGHT + y] = density;
                            density += bottom_increment;
                        }

                        back += back_increment;
                        front += front_increment;
                    }

                    back_left += back_left_increment;
                    front_left += front_left_increment;
                    back_right += back_right_increment;
                    front_right += front_right_increment;
                }
            }
        }
    }

    return result;
}

// We interpolate from a 4x3x4 to 16x24x16. 24 because we need some of the blocks above the
// chunk to know if we need to place surface blocks. Note how it affects the noise
// frequency. It is effectively 4x(8x vertically) since we sample closer together.
//
// NOTE: This is useful beyond the performance increase.
// 1. 3d noise tends to create small floaters that don't look good.
// 2. Even with complex noise compositions it's very easy to perceive regularity in it.
//    This breaks it up, while providing better continuity.
fn interpolate(noise: &Vec<f32>) -> Vec<f32> {
    const WIDTH: usize = Chunk::SIZE / TERRAIN_WIDTH_FACTOR;
    const HEIGHT: usize = CHUNK_HEIGHT / TERRAIN_HEIGHT_FACTOR;
    const DEPTH: usize = WIDTH;
    const WIDTH_INCREMENT: f32 = 1.0 / TERRAIN_WIDTH_FACTOR as f32;
    const HEIGHT_INCREMENT: f32 = 1.0 / TERRAIN_HEIGHT_FACTOR as f32;

    fn index(x: usize, y: usize, z: usize) -> usize {
        return x * (DEPTH + 1) * (HEIGHT + 1) + z * (HEIGHT + 1) + y;
    }

    let mut result = vec![0.0; Chunk::SIZE * CHUNK_HEIGHT * Chunk::SIZE];

    for x_noise in 0..WIDTH {
        for z_noise in 0..DEPTH {
            for y_noise in 0..HEIGHT {
                let mut back_left = noise[index(x_noise + 0, y_noise + 0, z_noise + 0)];
                let mut front_left = noise[index(x_noise + 0, y_noise + 0, z_noise + 1)];
                let mut back_right = noise[index(x_noise + 1, y_noise + 0, z_noise + 0)];
                let mut front_right = noise[index(x_noise + 1, y_noise + 0, z_noise + 1)];
                let back_left_increment = (noise[index(x_noise + 0, y_noise + 1, z_noise + 0)]
                    - back_left)
                    * HEIGHT_INCREMENT;
                let front_left_increment = (noise[index(x_noise + 0, y_noise + 1, z_noise + 1)]
                    - front_left)
                    * HEIGHT_INCREMENT;
                let back_right_increment = (noise[index(x_noise + 1, y_noise + 1, z_noise + 0)]
                    - back_right)
                    * HEIGHT_INCREMENT;
                let front_right_increment = (noise[index(x_noise + 1, y_noise + 1, z_noise + 1)]
                    - front_right)
                    * HEIGHT_INCREMENT;

                for y_index in 0..TERRAIN_HEIGHT_FACTOR {
                    let y = y_noise * TERRAIN_HEIGHT_FACTOR + y_index;

                    let back_increment = (back_right - back_left) * WIDTH_INCREMENT;
                    let front_increment = (front_right - front_left) * WIDTH_INCREMENT;

                    let mut back = back_left;
                    let mut front = front_left;

                    for x_index in 0..TERRAIN_WIDTH_FACTOR {
                        let x = x_noise * WIDTH + x_index;

                        let bottom_increment = (front - back) * 0.25;
                        let mut density = back;

                        for z_index in 0..TERRAIN_WIDTH_FACTOR {
                            let z = z_noise * WIDTH + z_index;
                            result[x * Chunk::SIZE * CHUNK_HEIGHT + z * CHUNK_HEIGHT + y] = density;
                            density += bottom_increment;
                        }

                        back += back_increment;
                        front += front_increment;
                    }

                    back_left += back_left_increment;
                    front_left += front_left_increment;
                    back_right += back_right_increment;
                    front_right += front_right_increment;
                }
            }
        }
    }

    return result;
}
