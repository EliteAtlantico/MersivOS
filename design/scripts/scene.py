# Loop mockup scene. Run:
#   blender -b --factory-startup --python scene.py -- --out DIR [--res 1920x1200] [--samples 64] [--views overview,ahead,down]
# First-person views are rendered in two passes (room / UI) and composited by composite.py.
import argparse
import math
import os
import random
import sys

import bpy
from mathutils import Matrix, Vector

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
ap = argparse.ArgumentParser()
ap.add_argument("--out", required=True)
ap.add_argument("--res", default="1920x1200")
ap.add_argument("--samples", type=int, default=64)
ap.add_argument("--views", default="overview,ahead,down")
ap.add_argument("--save-blend", default="")
args = ap.parse_args(argv)

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
FONT_R = bpy.data.fonts.load(os.path.join(ROOT, "assets/fonts/AtkinsonHyperlegible-Regular.ttf"))
FONT_B = bpy.data.fonts.load(os.path.join(ROOT, "assets/fonts/AtkinsonHyperlegible-Bold.ttf"))

EYE = Vector((0.0, 0.0, 1.65))
RING_R, RING_Z, TILE_Z = 0.70, 1.00, 1.08
ARC_R, ARC_Z = 0.60, 0.965

C_SURFACE = "#0B0B0C"
C_TEXT = "#F2F2F2"
C_TEXT2 = "#C9C9C9"
C_GHOST = "#BDBDBD"
C_REC = "#FF7B6B"
C_LISTEN = "#6FE3FF"
C_NEEDS = "#FFC857"
C_ADAPT = "#C9A7FF"

random.seed(7)

for ob in list(bpy.data.objects):
    bpy.data.objects.remove(ob, do_unlink=True)
scene = bpy.context.scene


def collection(name):
    col = bpy.data.collections.new(name)
    scene.collection.children.link(col)
    return col


COL_ROOM, COL_UI, COL_PERSON = collection("room"), collection("ui"), collection("person")


def lin(hexstr):
    c = [int(hexstr[i:i + 2], 16) / 255 for i in (1, 3, 5)]
    return tuple(x / 12.92 if x <= 0.04045 else ((x + 0.055) / 1.055) ** 2.4 for x in c) + (1.0,)


def new_material(name):
    mat = bpy.data.materials.new(name)
    try:
        mat.use_nodes = True
    except Exception:
        pass
    mat.node_tree.nodes.clear()
    return mat, mat.node_tree.nodes, mat.node_tree.links


def gray_mat(name, g, rough=0.75):
    mat, nodes, links = new_material(name)
    out = nodes.new("ShaderNodeOutputMaterial")
    bsdf = nodes.new("ShaderNodeBsdfPrincipled")
    bsdf.inputs["Base Color"].default_value = (g, g, g, 1)
    bsdf.inputs["Roughness"].default_value = rough
    links.new(bsdf.outputs[0], out.inputs["Surface"])
    return mat


def wood_mat(name):
    mat, nodes, links = new_material(name)
    out = nodes.new("ShaderNodeOutputMaterial")
    bsdf = nodes.new("ShaderNodeBsdfPrincipled")
    tc = nodes.new("ShaderNodeTexCoord")
    mp = nodes.new("ShaderNodeMapping")
    mp.inputs["Scale"].default_value = (0.6, 9.0, 1.0)
    wave = nodes.new("ShaderNodeTexNoise")
    wave.inputs["Scale"].default_value = 4.0
    wave.inputs["Detail"].default_value = 6.0
    ramp = nodes.new("ShaderNodeValToRGB")
    ramp.color_ramp.elements[0].color = (0.16, 0.16, 0.16, 1)
    ramp.color_ramp.elements[1].color = (0.30, 0.30, 0.30, 1)
    links.new(tc.outputs["Object"], mp.inputs["Vector"])
    links.new(mp.outputs[0], wave.inputs["Vector"])
    links.new(wave.outputs["Fac"], ramp.inputs["Fac"])
    links.new(ramp.outputs["Color"], bsdf.inputs["Base Color"])
    bsdf.inputs["Roughness"].default_value = 0.55
    links.new(bsdf.outputs[0], out.inputs["Surface"])
    return mat


_ui_mats = {}


def ui_mat(hexstr, alpha=1.0):
    key = (hexstr, alpha)
    if key in _ui_mats:
        return _ui_mats[key]
    mat, nodes, links = new_material(f"ui_{hexstr}_{alpha}")
    out = nodes.new("ShaderNodeOutputMaterial")
    em = nodes.new("ShaderNodeEmission")
    em.inputs["Color"].default_value = lin(hexstr)
    em.inputs["Strength"].default_value = 1.0
    tr = nodes.new("ShaderNodeBsdfTransparent")
    mix = nodes.new("ShaderNodeMixShader")
    mix.inputs["Fac"].default_value = alpha
    links.new(tr.outputs[0], mix.inputs[1])
    links.new(em.outputs[0], mix.inputs[2])
    links.new(mix.outputs[0], out.inputs["Surface"])
    mat.blend_method = "BLEND" if alpha < 1 else "OPAQUE"
    _ui_mats[key] = mat
    return mat


def link(ob, col):
    col.objects.link(ob)
    return ob


def box(name, center, size, mat, col=COL_ROOM, bevel=0.0):
    bpy.ops.mesh.primitive_cube_add(size=1, location=center)
    ob = bpy.context.active_object
    ob.name = name
    ob.scale = size
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    for c in ob.users_collection:
        c.objects.unlink(ob)
    col.objects.link(ob)
    ob.data.materials.append(mat)
    if bevel:
        m = ob.modifiers.new("bevel", "BEVEL")
        m.width = bevel
        m.segments = 3
    return ob


def cylinder(name, center, radius, depth, mat, col=COL_ROOM, verts=48, rot=(0, 0, 0)):
    bpy.ops.mesh.primitive_cylinder_add(vertices=verts, radius=radius, depth=depth, location=center, rotation=rot)
    ob = bpy.context.active_object
    ob.name = name
    for c in ob.users_collection:
        c.objects.unlink(ob)
    col.objects.link(ob)
    ob.data.materials.append(mat)
    bpy.ops.object.shade_smooth()
    return ob


def sphere(name, center, radius, mat, col=COL_ROOM, scale=(1, 1, 1)):
    bpy.ops.mesh.primitive_uv_sphere_add(segments=48, ring_count=24, radius=radius, location=center)
    ob = bpy.context.active_object
    ob.name = name
    ob.scale = scale
    for c in ob.users_collection:
        c.objects.unlink(ob)
    col.objects.link(ob)
    ob.data.materials.append(mat)
    bpy.ops.object.shade_smooth()
    return ob


# ---------------------------------------------------------------- room

M_WALL = gray_mat("wall", 0.5, 0.9)
M_FLOOR = wood_mat("floor")
M_CEIL = gray_mat("ceiling", 0.7, 0.9)
M_WOOD = gray_mat("wood", 0.38, 0.5)
M_DARK = gray_mat("dark", 0.06, 0.4)
M_METAL = gray_mat("metal", 0.5, 0.3)
M_FABRIC = gray_mat("fabric", 0.45, 0.95)
M_RUG = gray_mat("rug", 0.42, 1.0)
M_PLANT = gray_mat("plant", 0.22, 0.6)
M_SKIN = gray_mat("mannequin", 0.58, 0.45)

W, H, T = 2.6, 2.7, 0.1
box("floor", (0, 0, -0.025), (2 * W, 2 * W, 0.05), M_FLOOR)
box("ceiling", (0, 0, H + 0.025), (2 * W, 2 * W, 0.05), M_CEIL)
box("wall_back", (0, W + T / 2, H / 2), (2 * W + 2 * T, T, H), M_WALL)
box("wall_front", (0, -W - T / 2, H / 2), (2 * W + 2 * T, T, H), M_WALL)
# left wall with a window (y 0.2..1.6, z 0.9..2.1)
wy0, wy1, wz0, wz1 = 0.2, 1.6, 0.9, 2.1
box("wall_left_low", (-W - T / 2, 0, wz0 / 2), (T, 2 * W, wz0), M_WALL)
box("wall_left_high", (-W - T / 2, 0, (wz1 + H) / 2), (T, 2 * W, H - wz1), M_WALL)
box("wall_left_a", (-W - T / 2, (-W + wy0) / 2, (wz0 + wz1) / 2), (T, wy0 + W, wz1 - wz0), M_WALL)
box("wall_left_b", (-W - T / 2, (wy1 + W) / 2, (wz0 + wz1) / 2), (T, W - wy1, wz1 - wz0), M_WALL)
for y in (wy0, (wy0 + wy1) / 2, wy1):
    box("mullion", (-W - T / 2, y, (wz0 + wz1) / 2), (T * 1.2, 0.04, wz1 - wz0), M_WOOD)
for z in (wz0, wz1):
    box("sill", (-W - T / 2 + 0.03, (wy0 + wy1) / 2, z), (T * 1.6, wy1 - wy0 + 0.04, 0.04), M_WOOD)
# right wall with a doorway (y -0.9..0.0, z 0..2.1)
dy0, dy1, dz = -0.9, 0.0, 2.1
box("wall_right_a", (W + T / 2, (-W + dy0) / 2, H / 2), (T, dy0 + W, H), M_WALL)
box("wall_right_b", (W + T / 2, (dy1 + W) / 2, H / 2), (T, W - dy1, H), M_WALL)
box("wall_right_top", (W + T / 2, (dy0 + dy1) / 2, (dz + H) / 2), (T, dy1 - dy0, H - dz), M_WALL)
box("hall_back", (W + 1.4, (dy0 + dy1) / 2, H / 2), (0.05, 2.0, H), gray_mat("hall", 0.25))
box("hall_floor", (W + 0.7, (dy0 + dy1) / 2, -0.02), (1.4, 2.0, 0.04), M_FLOOR)
for y in (dy0, dy1):
    box("door_jamb", (W, y, dz / 2), (T * 1.3, 0.05, dz), M_WOOD)
box("door_head", (W, (dy0 + dy1) / 2, dz), (T * 1.3, dy1 - dy0 + 0.05, 0.05), M_WOOD)
# skirting
for name, c, s in (("skirt_b", (0, W - 0.01, 0.05), (2 * W, 0.02, 0.1)),
                   ("skirt_l1", (-W + 0.01, 0, 0.05), (0.02, 2 * W, 0.1))):
    box(name, c, s, M_WOOD)

# desk against the back wall, left
dx, dyc = -1.25, 2.22
box("desk_top", (dx, dyc, 0.74), (1.4, 0.7, 0.04), M_WOOD, bevel=0.006)
for sx in (-0.66, 0.66):
    for sy in (-0.31, 0.31):
        box("desk_leg", (dx + sx, dyc + sy, 0.36), (0.04, 0.04, 0.72), M_METAL)
box("laptop_base", (dx - 0.15, dyc - 0.05, 0.768), (0.34, 0.23, 0.016), M_METAL, bevel=0.004)
lap = box("laptop_lid", (dx - 0.15, dyc + 0.075, 0.88), (0.34, 0.012, 0.23), M_DARK, bevel=0.004)
lap.rotation_euler = (math.radians(-12), 0, 0)
cylinder("mug", (dx + 0.35, dyc - 0.12, 0.81), 0.04, 0.1, gray_mat("mug", 0.85, 0.2))
for i in range(4):
    box("book_stack", (dx + 0.48, dyc + 0.12, 0.775 + i * 0.032), (0.22 - i * 0.01, 0.16, 0.03),
        gray_mat(f"bk{i}", 0.2 + 0.15 * i), bevel=0.003)
cylinder("lamp_base", (dx - 0.55, dyc + 0.18, 0.77), 0.07, 0.02, M_METAL)
cylinder("lamp_arm", (dx - 0.55, dyc + 0.18, 1.0), 0.008, 0.45, M_METAL)
cylinder("lamp_head", (dx - 0.47, dyc + 0.14, 1.2), 0.06, 0.1, M_DARK, rot=(0, math.radians(60), 0))
# chair
cx, cy = -1.2, 1.55
box("chair_seat", (cx, cy, 0.46), (0.46, 0.44, 0.06), M_FABRIC, bevel=0.02)
box("chair_back", (cx, cy - 0.22, 0.78), (0.44, 0.05, 0.5), M_FABRIC, bevel=0.02)
cylinder("chair_post", (cx, cy, 0.23), 0.025, 0.42, M_METAL)
for a in range(5):
    t = a * 2 * math.pi / 5
    box("chair_foot", (cx + 0.16 * math.cos(t), cy + 0.16 * math.sin(t), 0.03), (0.3, 0.035, 0.03), M_METAL
        ).rotation_euler = (0, 0, t)
# bookshelf on the right wall
bx, by0, by1, bh = W - 0.17, 1.25, 2.35, 1.9
box("shelf_l", (bx, by0, bh / 2), (0.32, 0.03, bh), M_WOOD)
box("shelf_r", (bx, by1, bh / 2), (0.32, 0.03, bh), M_WOOD)
box("shelf_back", (bx + 0.15, (by0 + by1) / 2, bh / 2), (0.02, by1 - by0, bh), M_WOOD)
for k in range(6):
    z = 0.05 + k * (bh - 0.08) / 5
    box("shelf", (bx, (by0 + by1) / 2, z), (0.32, by1 - by0, 0.025), M_WOOD)
    if k == 5:
        continue
    y = by0 + 0.03
    while y < by1 - 0.08:
        wb = random.uniform(0.025, 0.05)
        hb = random.uniform(0.17, 0.27)
        if random.random() < 0.12:
            y += 0.06
            continue
        bk = box("book", (bx + 0.02, y + wb / 2, z + 0.0125 + hb / 2), (0.22, wb, hb),
                 gray_mat(f"book{random.random():.3f}", random.uniform(0.1, 0.75), 0.6), bevel=0.002)
        y += wb + 0.003
# plant
cylinder("pot", (2.2, 2.3, 0.18), 0.17, 0.36, gray_mat("pot", 0.8, 0.3))
for i in range(14):
    r = random.uniform(0.09, 0.16)
    sphere("leaf", (2.2 + random.uniform(-0.18, 0.18), 2.3 + random.uniform(-0.18, 0.18),
                    0.5 + random.uniform(0, 0.55)), r, M_PLANT, scale=(1, 1, 1.4))
# rug, wall art, ceiling lamp
box("rug", (0.1, 0.1, 0.005), (2.4, 1.7, 0.01), M_RUG, bevel=0.004)
box("art_frame", (-1.25, W - 0.02, 1.75), (0.7, 0.03, 0.5), M_DARK, bevel=0.004)
box("art", (-1.25, W - 0.04, 1.75), (0.6, 0.01, 0.4), gray_mat("art", 0.78))
cylinder("ceiling_lamp", (0, 0.4, H - 0.02), 0.22, 0.03, gray_mat("lampshade", 0.95))

# lights
def area_light(name, loc, rot, size, energy, size_y=None):
    ld = bpy.data.lights.new(name, "AREA")
    ld.energy = energy
    if size_y:
        ld.shape = "RECTANGLE"
        ld.size, ld.size_y = size, size_y
    else:
        ld.size = size
    ob = bpy.data.objects.new(name, ld)
    ob.location, ob.rotation_euler = loc, rot
    return link(ob, COL_ROOM)


area_light("window_sun", (-W - 0.6, (wy0 + wy1) / 2, 1.5), (0, math.radians(-90), 0), 1.4, 260, 1.2)
area_light("ceiling", (0, 0.4, H - 0.06), (0, 0, 0), 0.5, 45)
area_light("hall", (W + 0.8, (dy0 + dy1) / 2, H - 0.1), (0, 0, 0), 0.6, 20)
sky = box("sky", (-W - 1.2, 0.9, 1.5), (0.02, 4, 3), ui_mat("#D8D8D8"))

world = bpy.data.worlds.new("world")
scene.world = world
try:
    world.use_nodes = True
except Exception:
    pass
world.node_tree.nodes["Background"].inputs["Color"].default_value = (0.04, 0.04, 0.04, 1)

# ---------------------------------------------------------------- person (overview only)
sphere("head", (0, 0.0, 1.62), 0.105, M_SKIN, COL_PERSON, scale=(0.9, 1.0, 1.1))
cylinder("neck", (0, 0, 1.47), 0.05, 0.12, M_SKIN, COL_PERSON)
sphere("torso", (0, 0, 1.2), 0.2, M_SKIN, COL_PERSON, scale=(1.0, 0.62, 1.45))
sphere("hips", (0, 0, 0.93), 0.17, M_SKIN, COL_PERSON, scale=(1.0, 0.7, 0.7))
for sx in (-0.085, 0.085):
    cylinder("leg", (sx, 0, 0.45), 0.065, 0.9, M_SKIN, COL_PERSON)
for sx in (-1, 1):
    sphere("shoulder", (sx * 0.185, 0, 1.39), 0.058, M_SKIN, COL_PERSON)
    upper = cylinder("upper_arm", (sx * 0.205, 0.0, 1.22), 0.043, 0.34, M_SKIN, COL_PERSON)
    upper.rotation_euler = (0, sx * math.radians(7), 0)
    fore = cylinder("forearm", (sx * 0.22, 0.12, 1.0), 0.038, 0.28, M_SKIN, COL_PERSON)
    fore.rotation_euler = (math.radians(-110), 0, 0)
    sphere("hand", (sx * 0.22, 0.25, 0.955), 0.045, M_SKIN, COL_PERSON)
hs = box("headset", (0, 0.085, 1.64), (0.2, 0.11, 0.1), gray_mat("headset", 0.1, 0.35), COL_PERSON, bevel=0.03)
strap = cylinder("strap", (0, -0.005, 1.66), 0.112, 0.03, gray_mat("strap", 0.12, 0.9), COL_PERSON)
strap.scale = (0.95, 1.05, 1)

# ---------------------------------------------------------------- UI helpers


def rounded_rect(w, h, r, n=8):
    pts = []
    for cx, cy, a0 in ((w / 2 - r, h / 2 - r, 0), (-w / 2 + r, h / 2 - r, 90),
                       (-w / 2 + r, -h / 2 + r, 180), (w / 2 - r, -h / 2 + r, 270)):
        for i in range(n + 1):
            a = math.radians(a0 + 90 * i / n)
            pts.append((cx + r * math.cos(a), cy + r * math.sin(a)))
    return pts


def mesh_object(name, verts, faces, mat, col=COL_UI, parent=None, z=0.0):
    me = bpy.data.meshes.new(name)
    me.from_pydata([(x, y, z) for x, y in verts], [], faces)
    me.materials.append(mat)
    ob = bpy.data.objects.new(name, me)
    if parent:
        ob.parent = parent
    return link(ob, col)


def panel(name, w, h, r, mat, parent=None, z=0.0):
    pts = rounded_rect(w, h, r)
    return mesh_object(name, pts, [list(range(len(pts)))], mat, parent=parent, z=z)


def frame(name, w, h, r, width, mat, parent=None, z=0.0):
    outer = rounded_rect(w, h, r)
    inner = rounded_rect(w - 2 * width, h - 2 * width, max(r - width, 0.0005))
    n = len(outer)
    faces = [[i, (i + 1) % n, n + (i + 1) % n, n + i] for i in range(n)]
    return mesh_object(name, outer + inner, faces, mat, parent=parent, z=z)


def glyph(shape, size, hexstr, parent, loc):
    s = size / 2
    if shape == "dot":
        pts = [(s * math.cos(2 * math.pi * i / 40), s * math.sin(2 * math.pi * i / 40)) for i in range(40)]
        ob = mesh_object("g_dot", pts, [list(range(40))], ui_mat(hexstr), parent=parent)
    elif shape == "ring":
        outer = [(s * math.cos(2 * math.pi * i / 40), s * math.sin(2 * math.pi * i / 40)) for i in range(40)]
        inner = [(0.62 * x, 0.62 * y) for x, y in outer]
        faces = [[i, (i + 1) % 40, 40 + (i + 1) % 40, 40 + i] for i in range(40)]
        ob = mesh_object("g_ring", outer + inner, faces, ui_mat(hexstr), parent=parent)
    elif shape == "triangle":
        pts = [(0, s), (-s * 0.95, -s * 0.75), (s * 0.95, -s * 0.75)]
        ob = mesh_object("g_tri", pts, [[0, 1, 2]], ui_mat(hexstr), parent=parent)
    elif shape == "diamond":
        pts = [(0, s), (-s * 0.8, 0), (0, -s), (s * 0.8, 0)]
        ob = mesh_object("g_dia", pts, [[0, 1, 2, 3]], ui_mat(hexstr), parent=parent)
    ob.location = loc
    return ob


def text(body, size, hexstr, parent, loc, bold=False, align_x="LEFT", align_y="TOP", spacing=1.0):
    cu = bpy.data.curves.new("txt", "FONT")
    cu.body = body
    cu.font = FONT_B if bold else FONT_R
    cu.size = size
    cu.align_x, cu.align_y = align_x, align_y
    cu.space_line = spacing
    cu.materials.append(ui_mat(hexstr))
    ob = bpy.data.objects.new("txt", cu)
    ob.parent = parent
    ob.location = loc
    return link(ob, COL_UI)


def facing(pos, target=EYE):
    q = (target - pos).to_track_quat("Z", "Y")
    return Matrix.Translation(pos) @ q.to_matrix().to_4x4()


def empty(name, matrix):
    ob = bpy.data.objects.new(name, None)
    ob.matrix_world = matrix
    return link(ob, COL_UI)


# ---------------------------------------------------------------- ring

me = bpy.data.meshes.new("ring_line")
N = 256
verts = []
for i in range(N):
    a = 2 * math.pi * i / N
    for dr in (-0.0011, 0.0011):
        verts.append(((RING_R + dr) * math.sin(a), (RING_R + dr) * math.cos(a), RING_Z))
faces = [[2 * i, 2 * i + 1, 2 * ((i + 1) % N) + 1, 2 * ((i + 1) % N)] for i in range(N)]
me.from_pydata(verts, [], faces)
me.materials.append(ui_mat(C_TEXT2))
link(bpy.data.objects.new("ring_line", me), COL_UI)

TW, TH, TR = 0.20, 0.12, 0.014
PAD = 0.015
TILES = [
    (-45, "Notes", "Raised", "ghost", None),
    (-27, "Reader", "Ch. 4 · 62%", "focus", "listen"),
    (-9, "Web", "2 pages", "rest", None),
    (9, "Signals", "On the wall", "ghost", None),
    (27, "Apps", "12 Android", "rest", None),
    (45, "Settings", "All on device", "rest", None),
]
for ang, label, sub, state, mark in TILES:
    a = math.radians(ang)
    pos = Vector((RING_R * math.sin(a), RING_R * math.cos(a), TILE_Z))
    anchor = empty(f"tile_{label}", facing(pos))
    if state == "ghost":
        panel("surface", TW, TH, TR, ui_mat(C_SURFACE, 0.6), parent=anchor)
        frame("ghost", TW, TH, TR, 0.0022, ui_mat(C_GHOST), parent=anchor, z=0.001)
        tc, sc = C_TEXT2, C_TEXT2
    else:
        panel("surface", TW, TH, TR, ui_mat(C_SURFACE, 0.96), parent=anchor)
        tc, sc = C_TEXT, C_TEXT2
        if state == "focus":
            frame("focus", TW + 0.008, TH + 0.008, TR + 0.004, 0.0035, ui_mat("#FFFFFF"), parent=anchor)
    text(label, 0.032, tc, anchor, (-TW / 2 + PAD, TH / 2 - PAD, 0.002), bold=True)
    text(sub, 0.021, sc, anchor, (-TW / 2 + PAD, -TH / 2 + PAD, 0.002), align_y="BOTTOM")
    if mark == "listen":
        glyph("ring", 0.024, C_LISTEN, anchor, (TW / 2 - PAD - 0.008, TH / 2 - PAD - 0.008, 0.002))

# status arc: a row of segments along the front of the ring
ARC_ITEMS = [
    [("t", "14:32", C_TEXT, True)],
    [("t", "82%", C_TEXT, False)],
    [("t", "Muse S", C_TEXT, False), ("dots", 4, 3)],
    [("g", "diamond", C_ADAPT), ("t", "Sounds held · focus", C_TEXT, False)],
]
SEG_H, SEG_PAD, SEG_GAP, ARC_TXT = 0.046, 0.016, 0.012, 0.022
bpy.context.view_layer.update()


def measure(body, bold):
    cu = bpy.data.curves.new("m", "FONT")
    cu.body, cu.font, cu.size = body, FONT_B if bold else FONT_R, ARC_TXT
    ob = bpy.data.objects.new("m", cu)
    scene.collection.objects.link(ob)
    bpy.context.view_layer.update()
    wdt = ob.dimensions.x
    bpy.data.objects.remove(ob, do_unlink=True)
    return wdt


widths = []
for parts in ARC_ITEMS:
    wdt = 0.0
    for p in parts:
        if p[0] == "t":
            wdt += measure(p[1], p[3])
        elif p[0] == "dots":
            wdt += p[1] * 0.014
        else:
            wdt += 0.02
        wdt += 0.008
    widths.append(wdt - 0.008 + 2 * SEG_PAD)
total = sum(widths) + SEG_GAP * (len(widths) - 1)
s = -total / 2
for parts, wdt in zip(ARC_ITEMS, widths):
    a = (s + wdt / 2) / ARC_R
    s += wdt + SEG_GAP
    pos = Vector((ARC_R * math.sin(a), ARC_R * math.cos(a), ARC_Z))
    anchor = empty("arc_seg", facing(pos))
    panel("arc_surface", wdt, SEG_H, SEG_H / 2 - 0.004, ui_mat(C_SURFACE, 0.96), parent=anchor)
    x = -wdt / 2 + SEG_PAD
    for p in parts:
        if p[0] == "t":
            text(p[1], ARC_TXT, p[2], anchor, (x, 0, 0.002), bold=p[3], align_y="CENTER")
            x += measure(p[1], p[3]) + 0.008
        elif p[0] == "dots":
            for k in range(p[1]):
                if k < p[2]:
                    glyph("dot", 0.009, C_TEXT, anchor, (x + 0.007 + k * 0.014, 0, 0.002))
                else:
                    glyph("ring", 0.009, C_TEXT, anchor, (x + 0.007 + k * 0.014, 0, 0.002))
            x += p[1] * 0.014 + 0.008
        else:
            glyph(p[1], 0.018, p[2], anchor, (x + 0.01, 0, 0.002))
            x += 0.028

# raised item: Notes at reading position
RW, RH = 0.52, 0.36
rpos = Vector((0.0, 0.72, EYE.z - 0.72 * math.tan(math.radians(12))))
raised = empty("raised_notes", facing(rpos))
panel("surface", RW, RH, 0.02, ui_mat(C_SURFACE, 0.96), parent=raised)
text("Field notes", 0.034, C_TEXT, raised, (-RW / 2 + 0.028, RH / 2 - 0.026, 0.002), bold=True)
text("Thursday · 4 entries", 0.019, C_TEXT2, raised, (RW / 2 - 0.028, RH / 2 - 0.032, 0.002), align_x="RIGHT")
body = ("The ring stayed put when I walked to the desk.\n"
        "Recall from the kitchen felt instant.\n"
        "Muse lost AF7 twice. Re-seat it before the session.\n"
        "Try a lower ring for seated work, about 0.9 m.\n"
        "Clench to confirm: 23 of 24 today.")
text(body, 0.0205, C_TEXT, raised, (-RW / 2 + 0.028, RH / 2 - 0.085, 0.002), spacing=1.25)
mesh_object("rule", [(-RW / 2 + 0.028, -RH / 2 + 0.062), (RW / 2 - 0.028, -RH / 2 + 0.062),
                     (RW / 2 - 0.028, -RH / 2 + 0.0632), (-RW / 2 + 0.028, -RH / 2 + 0.0632)],
            [[0, 1, 2, 3]], ui_mat(C_TEXT2), parent=raised, z=0.002)
for i, word in enumerate(("Return", "Pin", "Undo")):
    text(word, 0.022, C_TEXT, raised, (-RW / 2 + 0.028 + i * 0.11, -RH / 2 + 0.028, 0.002), bold=True,
         align_y="CENTER")

# pinned item: Signals on the back wall
PW, PH = 0.95, 0.56
ppos = Vector((1.42, W - 0.012, 1.6))
pinned = empty("pinned_signals", Matrix.Translation(ppos) @ Vector((0, -1, 0)).to_track_quat("Z", "Y").to_matrix().to_4x4())
panel("surface", PW, PH, 0.025, ui_mat(C_SURFACE, 0.96), parent=pinned)
text("Signals", 0.05, C_TEXT, pinned, (-PW / 2 + 0.04, PH / 2 - 0.035, 0.002), bold=True)
text("Muse S · 256 Hz · 4 of 4 good", 0.028, C_TEXT2, pinned, (-PW / 2 + 0.04, PH / 2 - 0.105, 0.002))
text("Not recording", 0.028, C_TEXT2, pinned, (PW / 2 - 0.04, PH / 2 - 0.05, 0.002), align_x="RIGHT")
for row, ch in enumerate(("TP9", "AF7", "AF8", "TP10")):
    y0 = PH / 2 - 0.20 - row * 0.085
    text(ch, 0.028, C_TEXT2, pinned, (-PW / 2 + 0.04, y0, 0.002), align_y="CENTER")
    cu = bpy.data.curves.new("trace", "CURVE")
    cu.dimensions = "3D"
    cu.bevel_depth = 0.0016
    cu.materials.append(ui_mat(C_TEXT))
    sp = cu.splines.new("POLY")
    n = 420
    sp.points.add(n - 1)
    ph = [random.uniform(0, 6.28) for _ in range(4)]
    for i in range(n):
        t = i / n * 4.0
        v = (0.45 * math.sin(2 * math.pi * 10.2 * t + ph[0]) * (0.6 + 0.4 * math.sin(2 * math.pi * 0.7 * t + ph[1]))
             + 0.3 * math.sin(2 * math.pi * 3.1 * t + ph[2]) + 0.25 * math.sin(2 * math.pi * 21 * t + ph[3])
             + random.gauss(0, 0.12))
        x = -PW / 2 + 0.15 + (PW - 0.19) * i / (n - 1)
        sp.points[i].co = (x, y0 + 0.022 * v, 0.003, 1)
    ob = bpy.data.objects.new("trace", cu)
    ob.parent = pinned
    link(ob, COL_UI)

# ---------------------------------------------------------------- render

rx, ry = (int(v) for v in args.res.split("x"))
scene.render.engine = "CYCLES"
scene.cycles.device = "CPU"
scene.cycles.samples = args.samples
scene.cycles.use_denoising = True
scene.render.resolution_x, scene.render.resolution_y = rx, ry
scene.render.resolution_percentage = 100
scene.view_settings.view_transform = "Standard"
scene.view_settings.look = "None"
scene.view_settings.exposure = 0.0
scene.render.image_settings.file_format = "PNG"
scene.render.image_settings.color_mode = "RGBA"


def camera(name, loc, target=None, pitch=None, yaw=0.0, hfov=60.0):
    cd = bpy.data.cameras.new(name)
    cd.lens_unit = "FOV"
    cd.sensor_fit = "HORIZONTAL"
    cd.angle = math.radians(hfov)
    cd.clip_start = 0.02
    ob = bpy.data.objects.new(name, cd)
    ob.location = loc
    if target is not None:
        ob.rotation_euler = (Vector(target) - Vector(loc)).to_track_quat("-Z", "Y").to_euler()
    else:
        ob.rotation_euler = (math.radians(90 + pitch), 0, math.radians(yaw))
    return link(ob, scene.collection)


VIEWS = {
    "overview": dict(cam=camera("cam_overview", (1.5, -1.15, 2.35), target=(-0.05, 0.45, 1.08), hfov=60), fp=False),
    "ahead": dict(cam=camera("cam_ahead", EYE + Vector((0, 0.03, 0)), pitch=-12, hfov=100), fp=True),
    "down": dict(cam=camera("cam_down", EYE + Vector((0, 0.03, 0)), pitch=-38, yaw=6, hfov=96), fp=True),
}
# 360° room-only panorama from the eye, used by the PC prototype as fake grayscale passthrough
sky_cam = camera("cam_sky", EYE, pitch=0)
sky_cam.data.type = "PANO"
sky_cam.data.panorama_type = "EQUIRECTANGULAR"
VIEWS["sky"] = dict(cam=sky_cam, fp=True, room_only=True)

if args.save_blend:
    bpy.ops.wm.save_as_mainfile(filepath=args.save_blend)

os.makedirs(args.out, exist_ok=True)
for name in args.views.split(","):
    v = VIEWS[name]
    scene.camera = v["cam"]
    COL_PERSON.hide_render = v["fp"]
    if not v["fp"]:
        COL_ROOM.hide_render = COL_UI.hide_render = False
        scene.render.film_transparent = False
        scene.render.filepath = os.path.join(args.out, f"{name}.png")
        bpy.ops.render.render(write_still=True)
        continue
    COL_ROOM.hide_render, COL_UI.hide_render = False, True
    scene.render.film_transparent = False
    scene.render.filepath = os.path.join(args.out, f"{name}_room.png")
    bpy.ops.render.render(write_still=True)
    if v.get("room_only"):
        continue
    COL_ROOM.hide_render, COL_UI.hide_render = True, False
    scene.render.film_transparent = True
    scene.render.filepath = os.path.join(args.out, f"{name}_ui.png")
    bpy.ops.render.render(write_still=True)
