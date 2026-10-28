# Turns the room pass into a grayscale "passthrough" image (camera softness, grain, vignette),
# lays the UI pass over it, and captions every view.
#   python composite.py RENDER_DIR OUT_DIR
import os
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
src, dst = sys.argv[1], sys.argv[2]
os.makedirs(dst, exist_ok=True)
rng = np.random.default_rng(3)

CAPTIONS = {
    "overview": "Loop · the ring stays where you left it · one item raised, one pinned to the wall",
    "ahead": "Loop · first person, looking ahead · grayscale passthrough with the raised item and the wall pin",
    "down": "Loop · first person, glancing down · the ring, focus outline, cyan = listening, status arc below",
}


def passthrough(img):
    g = img.convert("L").filter(ImageFilter.GaussianBlur(0.8))
    a = np.asarray(g, dtype=np.float32) / 255.0
    a = 0.05 + 0.9 * a
    h, w = a.shape
    a += rng.normal(0, 0.022, a.shape).astype(np.float32) * (0.6 + 0.4 * (1 - a))
    yy, xx = np.mgrid[0:h, 0:w]
    r = np.hypot((xx - w / 2) / (w / 2), (yy - h / 2) / (h / 2))
    a *= 1 - 0.28 * np.clip(r - 0.55, 0, None) ** 1.6
    a = np.clip(a, 0, 1)
    return Image.fromarray((a * 255).astype(np.uint8)).convert("RGBA")


def caption(img, text):
    w, h = img.size
    font = ImageFont.truetype(os.path.join(ROOT, "assets/fonts/AtkinsonHyperlegible-Regular.ttf"), max(14, h // 48))
    d = ImageDraw.Draw(img)
    x, y = h // 40, h - h // 40 - font.size
    tw = d.textlength(text, font=font)
    d.rounded_rectangle((x - 12, y - 10, x + tw + 12, y + font.size + 12), radius=10, fill=(11, 11, 12, 235))
    d.text((x, y), text, font=font, fill=(242, 242, 242, 255))
    return img


for name, cap in CAPTIONS.items():
    single = os.path.join(src, f"{name}.png")
    if os.path.exists(single):
        out = Image.open(single).convert("RGBA")
    elif os.path.exists(os.path.join(src, f"{name}_room.png")):
        room = passthrough(Image.open(os.path.join(src, f"{name}_room.png")))
        ui = Image.open(os.path.join(src, f"{name}_ui.png")).convert("RGBA")
        out = Image.alpha_composite(room, ui)
    else:
        continue
    caption(out, cap).convert("RGB").save(os.path.join(dst, f"loop_{name}.png"))
    print("wrote", os.path.join(dst, f"loop_{name}.png"))
