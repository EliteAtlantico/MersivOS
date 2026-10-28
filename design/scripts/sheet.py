# Design-language sheet: tile states, meaning colors, type, status arc. Drawn over the grayscale room pass.
#   python sheet.py ROOM_PASS_PNG OUT_PNG
import os
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
FR = os.path.join(ROOT, "assets/fonts/AtkinsonHyperlegible-Regular.ttf")
FB = os.path.join(ROOT, "assets/fonts/AtkinsonHyperlegible-Bold.ttf")

SURFACE = (11, 11, 12)
TEXT, TEXT2, GHOST = (242, 242, 242), (201, 201, 201), (189, 189, 189)
MEANING = [
    ("Recording", (255, 123, 107), "dot", "#FF7B6B"),
    ("Listening", (111, 227, 255), "ring", "#6FE3FF"),
    ("Needs you", (255, 200, 87), "triangle", "#FFC857"),
    ("Loop adapted", (201, 167, 255), "diamond", "#C9A7FF"),
]


def lum(c):
    c = [x / 255 for x in c]
    c = [x / 12.92 if x <= 0.04045 else ((x + 0.055) / 1.055) ** 2.4 for x in c]
    return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]


def contrast(a, b=SURFACE):
    la, lb = sorted((lum(a), lum(b)), reverse=True)
    return (la + 0.05) / (lb + 0.05)


def font(size, bold=False):
    return ImageFont.truetype(FB if bold else FR, size)


bg = Image.open(sys.argv[1]).convert("L").resize((1920, 1200)).filter(ImageFilter.GaussianBlur(3))
a = np.asarray(bg, dtype=np.float32) / 255 * 0.85 + 0.05
a += np.random.default_rng(1).normal(0, 0.02, a.shape)
img = Image.fromarray((np.clip(a, 0, 1) * 255).astype(np.uint8)).convert("RGBA")
ov = Image.new("RGBA", img.size, (0, 0, 0, 0))
d = ImageDraw.Draw(ov)


def glyph(shape, cx, cy, s, color):
    if shape == "dot":
        d.ellipse((cx - s, cy - s, cx + s, cy + s), fill=color)
    elif shape == "ring":
        d.ellipse((cx - s, cy - s, cx + s, cy + s), outline=color, width=max(2, int(s * 0.38)))
    elif shape == "triangle":
        d.polygon([(cx, cy - s), (cx - s * 0.95, cy + s * 0.75), (cx + s * 0.95, cy + s * 0.75)], fill=color)
    elif shape == "diamond":
        d.polygon([(cx, cy - s), (cx - s * 0.8, cy), (cx, cy + s), (cx + s * 0.8, cy)], fill=color)


def tile(x, y, w, h, label, sub, state="rest", mark=None, decision=None):
    r = 22
    if state == "ghost":
        d.rounded_rectangle((x, y, x + w, y + h), r, fill=SURFACE + (153,), outline=GHOST, width=3)
        tc = TEXT2
    else:
        d.rounded_rectangle((x, y, x + w, y + h), r, fill=SURFACE + (245,))
        tc = TEXT
        if state == "focus":
            d.rounded_rectangle((x - 7, y - 7, x + w + 7, y + h + 7), r + 6, outline=(255, 255, 255), width=5)
    d.text((x + 24, y + 20), label, font=font(40, True), fill=tc)
    if decision:
        d.text((x + 24, y + 82), decision[0], font=font(26), fill=TEXT)
        d.text((x + 24, y + h - 50), decision[1], font=font(28, True), fill=TEXT)
        d.text((x + 24 + d.textlength(decision[1], font=font(28, True)) + 40, y + h - 50), decision[2],
               font=font(28, True), fill=TEXT)
    else:
        d.text((x + 24, y + h - 52), sub, font=font(28), fill=TEXT2)
    if mark:
        name, col, shape, _ = mark
        glyph(shape, x + w - 36, y + 40, 14, col)


def caption(x, y, text):
    f = font(26)
    d.rounded_rectangle((x - 12, y - 6, x + d.textlength(text, font=f) + 12, y + 36), 12, fill=SURFACE + (225,))
    d.text((x, y), text, font=f, fill=TEXT)


def panel(x, y, w, h):
    d.rounded_rectangle((x, y, x + w, y + h), 28, fill=SURFACE + (225,))


# title
panel(40, 36, 1840, 108)
d.text((80, 58), "Loop · design language", font=font(52, True), fill=TEXT)
sub = "Atkinson Hyperlegible · white on near-black · color only when it means something"
d.text((1840 - d.textlength(sub, font=font(24)), 76), sub, font=font(24), fill=TEXT2)

# tile states
TW, TH = 330, 200
y0 = 220
states = [
    ("Rest", dict(label="Web", sub="2 pages")),
    ("Focused (gaze)", dict(label="Reader", sub="Ch. 4 · 62%", state="focus")),
    ("Listening for confirm", dict(label="Reader", sub="Ch. 4 · 62%", state="focus", mark=MEANING[1])),
    ("Needs you, inline", dict(label="Signals", sub="", mark=MEANING[2],
                               decision=("Delete today's recording?", "Keep", "Delete"))),
    ("Raised / pinned (ghost)", dict(label="Notes", sub="Raised", state="ghost")),
]
for i, (name, kw) in enumerate(states):
    x = 70 + i * (TW + 40)
    caption(x, y0 - 56, name)
    tile(x, y0, TW, TH, **kw)

# meaning colors
y1 = 520
panel(40, y1, 900, 400)
d.text((80, y1 + 30), "Color means something", font=font(36, True), fill=TEXT)
d.text((80, y1 + 78), "Always paired with a shape, so color is never the only cue.", font=font(24), fill=TEXT2)
for i, (name, col, shape, hexs) in enumerate(MEANING):
    yy = y1 + 140 + i * 62
    glyph(shape, 105, yy + 18, 16, col)
    d.text((140, yy), name, font=font(32, True), fill=col)
    d.text((420, yy + 4), f"{hexs}   {contrast(col):.1f} : 1 on tile", font=font(26), fill=TEXT2)

# type + roles
panel(980, y1, 900, 400)
d.text((1020, y1 + 30), "Type and text roles", font=font(36, True), fill=TEXT)
d.text((1020, y1 + 100), "Label · bold", font=font(44, True), fill=TEXT)
d.text((1020, y1 + 165), "Body text · regular, primary #F2F2F2", font=font(30), fill=TEXT)
d.text((1020, y1 + 215), f"Secondary · #C9C9C9 · {contrast(TEXT2):.1f} : 1", font=font(30), fill=TEXT2)
d.text((1020, y1 + 270), f"Primary on tile · {contrast(TEXT):.1f} : 1", font=font(26), fill=TEXT2)
d.text((1020, y1 + 310), "No dark gray or dark red text, in any state.", font=font(26), fill=TEXT2)
d.text((1020, y1 + 350), "Disabled = a word (\"off\"), never dimmed.", font=font(26), fill=TEXT2)

# status arc examples
y2 = 980
caption(70, y2 - 56, "Status arc segments (front of the ring)")
x = 70
for parts in (["14:32"], ["82%"], ["Muse S", ("dots", 3)], [MEANING[3], "Sounds held · focus"],
              [MEANING[0], "Recording"], [MEANING[1], "Talk"]):
    wdt = 48
    for p in parts:
        if isinstance(p, str):
            wdt += d.textlength(p, font=font(30)) + 14
        elif isinstance(p, tuple) and p[0] == "dots":
            wdt += 4 * 22 + 14
        else:
            wdt += 36
    wdt -= 14
    d.rounded_rectangle((x, y2, x + wdt, y2 + 66), 33, fill=SURFACE + (245,))
    cx = x + 24
    for p in parts:
        if isinstance(p, str):
            d.text((cx, y2 + 15), p, font=font(30, p == "14:32"), fill=TEXT)
            cx += d.textlength(p, font=font(30)) + 14
        elif isinstance(p, tuple) and p[0] == "dots":
            for k in range(4):
                glyph("dot" if k < p[1] else "ring", cx + 8 + k * 22, y2 + 33, 7, TEXT)
            cx += 4 * 22 + 14
        else:
            glyph(p[2], cx + 12, y2 + 33, 13, p[1])
            cx += 36
    x += wdt + 18
caption(70, y2 + 92, "Notifications are sound + controller buzz only. Confirming a lilac mark reverses that adaptation.")

Image.alpha_composite(img, ov).convert("RGB").save(sys.argv[2])
print("wrote", sys.argv[2])
