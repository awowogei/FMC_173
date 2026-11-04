import os
import json
import shutil
import copy
import itertools

base_name = "water"
material_name = "water"
friction = [5.0, 5.0, 5.0]
flowing_texture_name = "flowing_water.png"
still_texture_name = "still_water.png"
fog = {
    "color": {
        "LinearRgba": {
            "red": 0,
            "green": 0,
            "blue": 0.7,
            "alpha": 1
        }
    },
    "start": 0,
    "stop": 100
}

level_to_pixels = {
    1: 1,
    2: 2,
    3: 4,
    4: 6,
    5: 8,
    6: 10,
    7: 12,
    8: 14,
    9: 15,
    10: 16,
}


def level_height(level):
    return level_to_pixels[level] / 16


def make_block(name, top_quad, top_texture, rotate_texture, cull_top=False, light_attenuation=0):
    quads = []

    #top
    quads.append({
        "vertices": copy.deepcopy(top_quad),
        "texture": top_texture,
        "rotate_texture": rotate_texture
    })

    if cull_top:
        quads[0]["cull_face"] = "top"

    #bottom
    quads.append({
        "vertices": [
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
        ],
        "texture": still_texture_name,
        "cull_face": "bottom"
    })

    #right
    quads.append({
        "vertices": [
            [1.0, top_quad[3][1], 1.0],
            [1.0, 0.0, 1.0],
            [1.0, top_quad[2][1], 0.0],
            [1.0, 0.0, 0.0]
        ],
        "texture": flowing_texture_name,
        "cull_face": "right"
    })

    #left
    quads.append({
        "vertices": [
            [0.0, top_quad[0][1], 0.0],
            [0.0, 0.0, 0.0],
            [0.0, top_quad[1][1], 1.0],
            [0.0, 0.0, 1.0]
        ],
        "texture": flowing_texture_name,
        "cull_face": "left"
    })

    #front
    quads.append({
        "vertices": [
            [0.0, top_quad[1][1], 1.0],
            [0.0, 0.0, 1.0],
            [1.0, top_quad[3][1], 1.0],
            [1.0, 0.0, 1.0]
        ],
        "texture": flowing_texture_name,
        "cull_face": "front"
    })

    #back
    quads.append({
        "vertices": [
            [1.0, top_quad[2][1], 0.0],
            [1.0, 0.0, 0.0],
            [0.0, top_quad[0][1], 0.0],
            [0.0, 0.0, 0.0]
        ],
        "texture": flowing_texture_name,
        "cull_face": "back"
    })

    return {
        "parent": base_name + "/parent.json",
        "light_attenuation": light_attenuation,
        "name": name,
        "quads": quads
    }


def flat_quad(height):
    return [
        [0.0, height, 0.0],
        [0.0, height, 1.0],
        [1.0, height, 0.0],
        [1.0, height, 1.0],
    ]


def rotate(corners):
    (back_right, back_left, front_right, front_left) = corners
    return (front_right, back_right, front_left, back_left)


def slopes(corners):
    (back_right, back_left, front_right, front_left) = corners
    left_right = (front_right + back_right) - (front_left + back_left)
    back_front = (front_left + front_right) - (back_left + back_right)
    return (left_right, back_front)


def canonical(corners):
    rotations = [corners]
    for _ in range(3):
        rotations.append(rotate(rotations[-1]))

    def flows_forwards(corners):
        (left_right, back_front) = slopes(corners)
        return (left_right >= 0 and back_front <= 0, -back_front, corners)

    return max(rotations, key=flows_forwards)


def make_shapes():
    blocks = []

    for corners in itertools.product(range(1, 11), repeat=4):
        if canonical(corners) != corners:
            continue

        # Never occurs naturally from spreading, only present as the last stage
        # of decay, doesn't look good.
        if corners == (1, 1, 1, 1):
            continue

        # The surface of a block may drop at most 5 levels
        if max(corners) - min(corners) > 5:
            continue

        (back_right, back_left, front_right, front_left) = corners

        name = base_name + "_" + "_".join(str(level) for level in corners)

        top_quad = [
            [0.0, level_height(back_left), 0.0],
            [0.0, level_height(front_left), 1.0],
            [1.0, level_height(back_right), 0.0],
            [1.0, level_height(front_right), 1.0],
        ]

        # The still texture for surfaces without a slope, the flowing texture
        # for the rest.
        (left_right_slope, back_front_slope) = slopes(corners)
        if left_right_slope == 0 and back_front_slope == 0:
            (top_texture, rotate_texture) = (still_texture_name, False)
        elif left_right_slope == 0 or back_front_slope == 0:
            (top_texture, rotate_texture) = (flowing_texture_name, False)
        else:
            (top_texture, rotate_texture) = (flowing_texture_name, True)

        # The flat full block is the only shape that can sit directly below other
        # water, so its top face can be culled.
        cull_top = corners == (10, 10, 10, 10)

        blocks.append(make_block(name, top_quad, top_texture, rotate_texture, cull_top))

    for level in range(2, 10):
        name = base_name + "_" + "_".join(str(level) for _ in range(4)) + "_straight"
        top_quad = flat_quad(level_height(level))
        blocks.append(make_block(name, top_quad, flowing_texture_name, False))

    return blocks


blocks = []

blocks.append(make_block(
    name="ocean_bottom",
    top_quad=flat_quad(1.0),
    top_texture=still_texture_name,
    rotate_texture=False,
    cull_top=True,
    light_attenuation=2,
))

blocks.append(make_block(
    name="ocean_top",
    top_quad=flat_quad(level_height(8)),
    top_texture=still_texture_name,
    rotate_texture=False,
    cull_top=False
))

blocks.extend(make_shapes())

shutil.rmtree(base_name, ignore_errors=True)
os.mkdir(base_name)

parent = {
    "type": "cube",
    "material": material_name,
    "friction": friction,
    "fog": fog,
    "replaceable": True,
    "placement": {
        "floor": True,
        "ceiling": True,
        "sides": True,
        "rotatable": False,
    },
}
path = base_name + "/parent.json"
with open(path, "w") as file:
    json.dump(parent, file, indent=4)
    file.write("\n")

for block in blocks:
    path = base_name + "/" + block["name"] + ".json"
    with open(path, "w") as file:
        json.dump(block, file, indent=4)
