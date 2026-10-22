#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Draws the README header docs/banner.png (1280x640, also usable as the GitHub social preview).

Needs Pillow. Usage: scripts/banner.py . docs/banner.png
"""

import sys

from PIL import Image, ImageDraw, ImageFilter, ImageFont

repo, out = sys.argv[1], sys.argv[2]
S = 2  # drawn at twice the size, scaled down for smooth edges
W, H = 1280 * S, 640 * S

semibold = lambda size: ImageFont.truetype(f"{repo}/assets/fonts/Inter-SemiBold.ttf", size * S)
regular = lambda size: ImageFont.truetype(f"{repo}/assets/fonts/Inter-Regular.ttf", size * S)

# Colors of the app's dark theme
BG_TOP = (17, 21, 30)
BG_BOTTOM = (27, 34, 48)
CARD = (30, 37, 52)
BORDER = (52, 62, 82)
TEXT = (236, 240, 247)
MUTED = (150, 162, 184)
ACCENT = (77, 141, 255)
ACCENT_SOFT = (36, 56, 96)
GREEN = (52, 199, 120)

img = Image.new("RGB", (W, H), BG_TOP)
d = ImageDraw.Draw(img)
for y in range(H):
    t = y / H
    d.line([(0, y), (W, y)], fill=tuple(int(a + (b - a) * t) for a, b in zip(BG_TOP, BG_BOTTOM)))

# A soft blue glow behind the drawing on the right
glow = Image.new("RGB", (W, H), (0, 0, 0))
ImageDraw.Draw(glow).ellipse([W * 0.55, H * 0.12, W * 1.05, H * 0.92], fill=(30, 60, 120))
glow = glow.filter(ImageFilter.GaussianBlur(120 * S))
img = Image.blend(img, Image.composite(glow, img, glow.convert("L")), 0.55)
d = ImageDraw.Draw(img)


def s(v):
    return int(v * S)


# Left: logo, name, sentence, systems
icon = Image.open(f"{repo}/assets/icon-1024.png").convert("RGBA").resize((s(112), s(112)), Image.LANCZOS)
img.paste(icon, (s(80), s(150)), icon)
d.text((s(214), s(152)), "TableCross", font=semibold(84), fill=TEXT)
d.text(
    (s(82), s(300)),
    "One mouse, keyboard and clipboard\nfor all the computers on your desk.",
    font=regular(34),
    fill=MUTED,
    spacing=s(12),
)

x = s(82)
for label in ["macOS", "Windows", "Linux", "Local network only", "Open source"]:
    font = regular(22)
    w = d.textlength(label, font=font)
    d.rounded_rectangle([x, s(440), x + w + s(32), s(484)], radius=s(22), fill=CARD, outline=BORDER, width=s(1))
    d.text((x + s(16), s(449)), label, font=font, fill=TEXT)
    x += w + s(44)


# Right: three screens like on the arrangement page, the pointer crossing them
def screen(x0, y0, w, h, highlight=False):
    d.rounded_rectangle([s(x0), s(y0), s(x0 + w), s(y0 + h)], radius=s(14), fill=CARD, outline=BORDER, width=s(2))
    inner = [s(x0 + 12), s(y0 + 12), s(x0 + w - 12), s(y0 + h - 12)]
    d.rounded_rectangle(inner, radius=s(8), fill=ACCENT_SOFT if highlight else (22, 28, 40),
                        outline=ACCENT if highlight else (70, 82, 104), width=s(3))


screen(780, 170, 200, 135, highlight=True)
screen(990, 170, 200, 135)
screen(990, 315, 200, 135)
# Where the pointer crosses
d.rounded_rectangle([s(983), s(182), s(987), s(293)], radius=s(2), fill=GREEN)
d.rounded_rectangle([s(1002), s(308), s(1178), s(312)], radius=s(2), fill=GREEN)

# The way of the pointer, dotted
path = [(880, 240), (930, 236), (985, 238), (1060, 250), (1100, 290), (1105, 330), (1090, 380)]
for (x0, y0), (x1, y1) in zip(path, path[1:]):
    steps = int(((x1 - x0) ** 2 + (y1 - y0) ** 2) ** 0.5 / 11)
    for i in range(steps):
        t = i / max(steps, 1)
        cx, cy = x0 + (x1 - x0) * t, y0 + (y1 - y0) * t
        d.ellipse([s(cx - 2.2), s(cy - 2.2), s(cx + 2.2), s(cy + 2.2)], fill=(200, 214, 240))

# A pointer at the end of the way
px, py = 1090, 380
arrow = [(0, 0), (0, 34), (9, 26), (15, 40), (21, 37), (15, 24), (27, 24)]
d.polygon([(s(px + ax), s(py + ay)) for ax, ay in arrow], fill=(255, 255, 255), outline=(20, 24, 32))
d.line([(s(px + ax), s(py + ay)) for ax, ay in arrow + [arrow[0]]], fill=(20, 24, 32), width=s(2))

# Keyboard and mouse under the first screen: they belong to that computer
kx, ky = 800, 330
d.rounded_rectangle([s(kx), s(ky), s(kx + 120), s(ky + 34)], radius=s(6), fill=CARD, outline=BORDER, width=s(2))
for row in range(2):
    for col in range(8):
        cx = kx + 10 + col * 13
        cy = ky + 7 + row * 12
        d.rounded_rectangle([s(cx), s(cy), s(cx + 9), s(cy + 8)], radius=s(2), fill=(70, 82, 104))
d.rounded_rectangle([s(kx + 135), s(ky - 2), s(kx + 160), s(ky + 36)], radius=s(12), fill=CARD, outline=BORDER, width=s(2))
d.line([s(kx + 147.5), s(ky + 4), s(kx + 147.5), s(ky + 14)], fill=(70, 82, 104), width=s(2))

img = img.resize((1280, 640), Image.LANCZOS)
img.save(out, optimize=True)
print("saved", out)
