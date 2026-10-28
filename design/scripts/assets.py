# Exports Loop's runtime meshes as .glb into crates/loop-ring/res/loop/.
#   blender -b --factory-startup --python assets.py
# Shapes are flat, in Blender's XZ plane facing -Y, so they face +Z (towards the viewer) in glTF/Stardust.
# Color and unlit are set at runtime through material parameters.
import math
import os

import bpy

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
OUT = os.path.join(ROOT, "crates/loop-ring/res/loop")
os.makedirs(OUT, exist_ok=True)


def clear():
    for ob in list(bpy.data.objects):
        bpy.data.objects.remove(ob, do_unlink=True)
    for me in list(bpy.data.meshes):
        bpy.data.meshes.remove(me)


def material(alpha):
    mat = bpy.data.materials.new(f"surface_{alpha}")
    bsdf = mat.node_tree.nodes.get("Principled BSDF")
    bsdf.inputs["Base Color"].default_value = (0.0035, 0.0035, 0.004, 1)
    bsdf.inputs["Alpha"].default_value = alpha
    if alpha < 1:
        mat.surface_render_method = "BLENDED"
    return mat


def rounded_rect(w, h, r, n=8):
    pts = []
    for cx, cy, a0 in ((w / 2 - r, h / 2 - r, 0), (-w / 2 + r, h / 2 - r, 90),
                       (-w / 2 + r, -h / 2 + r, 180), (w / 2 - r, -h / 2 + r, 270)):
        for i in range(n + 1):
            a = math.radians(a0 + 90 * i / n)
            pts.append((cx + r * math.cos(a), cy + r * math.sin(a)))
    return pts


def export(name, pts, alpha=1.0, fan=True):
    clear()
    verts = [(x, 0.0, y) for x, y in pts]
    if fan:
        verts.append((sum(p[0] for p in pts) / len(pts), 0.0, sum(p[1] for p in pts) / len(pts)))
        c = len(verts) - 1
        faces = [[c, (i + 1) % len(pts), i] for i in range(len(pts))]
    else:
        faces = [list(range(len(pts)))[::-1]]
    me = bpy.data.meshes.new("surface")
    me.from_pydata(verts, [], faces)
    me.materials.append(material(alpha))
    ob = bpy.data.objects.new("surface", me)
    bpy.context.scene.collection.objects.link(ob)
    bpy.ops.export_scene.gltf(filepath=os.path.join(OUT, f"{name}.glb"), export_format="GLB",
                              export_yup=True, export_apply=True, export_materials="EXPORT")


export("tile", rounded_rect(0.20, 0.12, 0.014))
export("tile_ghost", rounded_rect(0.20, 0.12, 0.014), alpha=0.6)
export("raised", rounded_rect(0.52, 0.36, 0.02))
export("pinned", rounded_rect(0.95, 0.56, 0.025))
for cm in range(5, 41):
    w = cm / 100
    export(f"pill_{cm:02d}", rounded_rect(w, 0.046, 0.019))
export("disc", [(0.5 * math.cos(2 * math.pi * i / 40), 0.5 * math.sin(2 * math.pi * i / 40)) for i in range(40)])
export("diamond", [(0, 0.5), (-0.4, 0), (0, -0.5), (0.4, 0)], fan=False)
export("triangle", [(0, 0.5), (-0.475, -0.375), (0.475, -0.375)], fan=False)
print("exported to", OUT)
