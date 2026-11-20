use gltf_json as json;
use image::{Rgba, Rgba32FImage};
use json::buffer::Stride;

use std::borrow::Cow;
use std::mem;
use std::path::{Path, PathBuf};

use clap::Parser;
use json::validation::Checked::Valid;

const VERTICES: [[[f32; 3]; 6]; 6] = [
    // Top
    [
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 1.0],
        [1.0, 1.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 1.0],
        [1.0, 1.0, 1.0],
    ],
    // Bottom
    [
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
    ],
    // Left
    [
        [0.0, 0.0, 1.0],
        [0.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0],
        [0.0, 1.0, 1.0],
        [0.0, 1.0, 0.0],
    ],
    // Right
    [
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [1.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 0.0],
        [1.0, 1.0, 1.0],
    ],
    // Front
    [
        [0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 0.0],
    ],
    // Back
    [
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [0.0, 1.0, 1.0],
    ],
];

#[derive(Parser)]
#[clap(about, long_about = None)]
struct Cli {
    /// File to operate on
    #[clap(value_parser)]
    file_path: PathBuf,
    #[clap(long, help = "Model to copy the animations from")]
    animations: Option<PathBuf>,
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

/// Calculate bounding coordinates of a list of vertices, used for the clipping distance of the model
fn bounding_coords(points: &[Vertex]) -> ([f32; 3], [f32; 3]) {
    let mut min = [0., 0., 0.];
    let mut max = [0., 0., 0.];

    for point in points {
        for i in 0..3 {
            min[i] = f32::min(min[i], point.position[i]);
            max[i] = f32::max(max[i], point.position[i]);
        }
    }
    (min, max)
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn align_to_multiple_of_four(n: &mut u32) {
    *n = (*n + 3) & !3;
}

fn vertices_to_bytes(vertices: &[Vertex]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vertices.len() * mem::size_of::<Vertex>());
    for vertex in vertices {
        for value in vertex.position.iter().chain(vertex.color.iter()) {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

fn convert(image: &Rgba32FImage) -> (json::Root, Vec<u8>) {
    let mut triangle_vertices = Vec::new();

    let mut add_face = |x: f32, y: f32, z: f32, vertices: &[[f32; 3]; 6], color: &Rgba<f32>| {
        for pos in vertices.iter() {
            triangle_vertices.push(Vertex {
                position: [pos[0] + x, pos[1] + y, pos[2] + z],
                color: [
                    srgb_to_linear(color[0]),
                    srgb_to_linear(color[1]),
                    srgb_to_linear(color[2]),
                ],
            })
        }
    };

    for (x, y, color) in image.enumerate_pixels() {
        // Center align model position, and compensate for iteration starting in the top left
        // corner, which is the minimum x value and the maximum y value.
        let model_x = x as f32 - image.width() as f32 / 2.0;
        let model_y = image.height() as f32 / 2.0 - y as f32 - 1.0;
        let model_z = -0.5;

        if color[3] == 0.0 {
            continue;
        }

        // Above pixel
        if y as i32 - 1 < 0 || image.get_pixel(x, y - 1)[3] == 0.0 {
            add_face(model_x, model_y, model_z, &VERTICES[0], color);
        }

        // Below pixel
        if y + 1 == image.height() || image.get_pixel(x, y + 1)[3] == 0.0 {
            add_face(model_x, model_y, model_z, &VERTICES[1], color);
        }

        // Left pixel
        if x as i32 - 1 < 0 || image.get_pixel(x - 1, y)[3] == 0.0 {
            add_face(model_x, model_y, model_z, &VERTICES[2], color);
        }

        // Right pixel
        if x + 1 == image.width() || image.get_pixel(x + 1, y)[3] == 0.0 {
            add_face(model_x, model_y, model_z, &VERTICES[3], color);
        }

        // Front and Back pixel
        if color[3] != 0.0 {
            add_face(model_x, model_y, model_z, &VERTICES[4], color);
            add_face(model_x, model_y, model_z, &VERTICES[5], color);
        }
    }

    let (min, max) = bounding_coords(&triangle_vertices);

    let buffer_length = triangle_vertices.len() * mem::size_of::<Vertex>();
    let buffer = json::Buffer {
        byte_length: buffer_length.into(),
        extensions: Default::default(),
        extras: Default::default(),
        name: None,
        uri: None,
    };

    let buffer_view = json::buffer::View {
        buffer: json::Index::new(0),
        byte_length: buffer.byte_length,
        byte_offset: None,
        byte_stride: Some(Stride(mem::size_of::<Vertex>())),
        extensions: Default::default(),
        extras: Default::default(),
        name: None,
        target: Some(Valid(json::buffer::Target::ArrayBuffer)),
    };

    let positions = json::Accessor {
        buffer_view: Some(json::Index::new(0)),
        byte_offset: Some(0_usize.into()),
        count: triangle_vertices.len().into(),
        component_type: Valid(json::accessor::GenericComponentType(
            json::accessor::ComponentType::F32,
        )),
        extensions: Default::default(),
        extras: Default::default(),
        type_: Valid(json::accessor::Type::Vec3),
        min: Some(json::Value::from(Vec::from(min))),
        max: Some(json::Value::from(Vec::from(max))),
        name: None,
        normalized: false,
        sparse: None,
    };

    let colors = json::Accessor {
        buffer_view: Some(json::Index::new(0)),
        byte_offset: Some((3 * mem::size_of::<f32>()).into()),
        count: triangle_vertices.len().into(),
        component_type: Valid(json::accessor::GenericComponentType(
            json::accessor::ComponentType::F32,
        )),
        extensions: Default::default(),
        extras: Default::default(),
        type_: Valid(json::accessor::Type::Vec3),
        min: None,
        max: None,
        name: None,
        normalized: false,
        sparse: None,
    };

    let primitive = json::mesh::Primitive {
        attributes: {
            let mut map = std::collections::BTreeMap::new();
            map.insert(Valid(json::mesh::Semantic::Positions), json::Index::new(0));
            map.insert(Valid(json::mesh::Semantic::Colors(0)), json::Index::new(1));
            map
        },
        extensions: Default::default(),
        extras: Default::default(),
        indices: None,
        material: None,
        mode: Valid(json::mesh::Mode::Triangles),
        targets: None,
    };

    let mesh = json::Mesh {
        extensions: Default::default(),
        extras: Default::default(),
        name: None,
        primitives: vec![primitive],
        weights: None,
    };

    let node = json::Node {
        camera: None,
        children: None,
        extensions: Default::default(),
        extras: Default::default(),
        matrix: None,
        mesh: Some(json::Index::new(0)),
        name: None,
        rotation: None,
        scale: None,
        translation: None,
        skin: None,
        weights: None,
    };

    let root = json::Root {
        accessors: vec![positions, colors],
        buffers: vec![buffer],
        buffer_views: vec![buffer_view],
        meshes: vec![mesh],
        nodes: vec![node],
        scenes: vec![json::Scene {
            extensions: Default::default(),
            extras: Default::default(),
            name: None,
            nodes: vec![json::Index::new(0)],
        }],
        ..Default::default()
    };

    return (root, vertices_to_bytes(&triangle_vertices));
}

fn copy_accessor(
    root: &mut json::Root,
    bin: &mut Vec<u8>,
    template: &json::Root,
    template_bin: &[u8],
    index: json::Index<json::Accessor>,
) -> json::Index<json::Accessor> {
    let mut accessor = template.accessors[index.value()].clone();
    let view = &template.buffer_views[accessor
        .buffer_view
        .expect("template accessor has no buffer view")
        .value()];

    let element_size = match (&accessor.component_type, &accessor.type_) {
        (Valid(component_type), Valid(type_)) => component_type.0.size() * type_.multiplicity(),
        _ => panic!("template has an invalid accessor"),
    };
    let start = view.byte_offset.map_or(0, |offset| offset.0) as usize
        + accessor.byte_offset.map_or(0, |offset| offset.0) as usize;
    let length = element_size * accessor.count.0 as usize;

    let byte_offset = bin.len();
    bin.extend_from_slice(&template_bin[start..start + length]);
    while bin.len() % 4 != 0 {
        bin.push(0);
    }

    root.buffer_views.push(json::buffer::View {
        buffer: json::Index::new(0),
        byte_length: length.into(),
        byte_offset: Some(byte_offset.into()),
        byte_stride: None,
        extensions: Default::default(),
        extras: Default::default(),
        name: None,
        target: None,
    });

    accessor.buffer_view = Some(json::Index::new(root.buffer_views.len() as u32 - 1));
    accessor.byte_offset = None;
    root.accessors.push(accessor);

    return json::Index::new(root.accessors.len() as u32 - 1);
}

fn copy_animations(root: &mut json::Root, bin: &mut Vec<u8>, template_path: &Path) {
    let template_bytes = std::fs::read(template_path).expect("I/O error");
    let template_glb =
        gltf::binary::Glb::from_slice(&template_bytes).expect("template is not a valid glb");
    let template = json::Root::from_slice(&template_glb.json).expect("template has invalid json");
    let template_bin = template_glb.bin.expect("template has no binary chunk");

    for template_animation in &template.animations {
        let mut animation = template_animation.clone();

        for sampler in &mut animation.samplers {
            sampler.input = copy_accessor(root, bin, &template, &template_bin, sampler.input);
            sampler.output = copy_accessor(root, bin, &template, &template_bin, sampler.output);
        }

        for channel in &mut animation.channels {
            channel.target.node = json::Index::new(0);
        }

        root.animations.push(animation);
    }

    root.buffers[0].byte_length = bin.len().into();
}

fn main() {
    let mut cli = Cli::parse();

    let image = image::open(&cli.file_path).expect("I/O error").to_rgba32f();

    let (mut root, mut bin) = convert(&image);

    if let Some(template_path) = &cli.animations {
        copy_animations(&mut root, &mut bin, template_path);
    }

    let json_string = json::serialize::to_string(&root).expect("Serialization error");
    let mut json_offset = json_string.len() as u32;

    align_to_multiple_of_four(&mut json_offset);

    let glb = gltf::binary::Glb {
        header: gltf::binary::Header {
            magic: *b"glTF",
            version: 2,
            length: json_offset + bin.len() as u32,
        },
        bin: Some(Cow::Owned(bin)),
        json: Cow::Owned(json_string.into_bytes()),
    };

    cli.file_path.set_extension("glb");
    let file = std::fs::File::create(&cli.file_path).expect("I/O error");
    glb.to_writer(file).expect("glTF binary output error");
}
