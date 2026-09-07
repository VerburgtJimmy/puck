"""Bake the hero sky band.

A wide, thin watercolor crown that bleeds off the top of the viewport: densest
under the nav (so the middle nav links can be knocked out in white), dissolving
into open paper at both margins and above the headline. Output is straight
(un-premultiplied) RGBA, so compositing it on a white page behaves like pigment
on paper -- no white box, no hard edge anywhere.
"""

import numpy as np
from PIL import Image

SRC = "/Users/jimmyverburgt/Developer/personal/puck/site/art-src/wash-sky-blue.png"
OUT = "/Users/jimmyverburgt/Developer/personal/puck/site/public/art/sky-band.webp"

# Crop skips the ragged painted top edge of the source (it would read as a hard
# line across the viewport) and keeps the zenith plus the cloud banks that sit
# on the left and right of the sheet.
CROP = (60, 118, 1480, 720)
W, H = 2400, 500

src = Image.open(SRC).convert("RGB").crop(CROP).resize((W, H), Image.LANCZOS)
rgb = np.asarray(src, dtype=np.float32) / 255.0

# Pigment / paper split: the min channel is how much white is mixed in, the rest
# is ink. Recombining over white reproduces the source exactly.
paper = rgb.min(axis=2)
ink = 1.0 - paper
pig = np.zeros_like(rgb)
for c in range(3):
    pig[..., c] = np.where(ink > 1e-4, (rgb[..., c] - paper) / np.maximum(ink, 1e-4), 0.0)

xs = np.linspace(0.0, 1.0, W, dtype=np.float32)[None, :]
ys = np.linspace(0.0, 1.0, H, dtype=np.float32)[:, None]


def smoothstep(a, b, t):
    t = np.clip((t - a) / (b - a), 0.0, 1.0)
    return t * t * (3.0 - 2.0 * t)


# Density carried by the source wash, re-levelled so clouds stay open.
d = np.clip((ink - 0.09) / 0.46, 0.0, 1.0) ** 1.15

# Bottom edge: a single slow swell across the width so it drifts instead of
# forming a symmetric peak (which reads as a mountain, not a sky).
bottom = 0.72 + 0.12 * np.sin(xs * 2 * np.pi * 0.72 + 1.1)
env_v = smoothstep(np.repeat(bottom, H, axis=0), 0.06, ys)

# Side margins: open paper under the brand and the GitHub link, deep blue across
# the middle of the nav.
xw = xs + 0.022 * np.sin(ys * 2 * np.pi * 0.8) + 0.010 * np.sin(ys * 2 * np.pi * 2.1)
env_h = smoothstep(0.020, 0.335, xw) * smoothstep(0.980, 0.665, xw)

# Paper grain: low-frequency blotching so the wash is never flat.
rng = np.random.default_rng(7)
grain = rng.random((H // 12 + 2, W // 12 + 2)).astype(np.float32)
grain = np.asarray(Image.fromarray((grain * 255).astype(np.uint8)).resize((W, H), Image.BICUBIC), dtype=np.float32) / 255.0
grain = 0.92 + 0.16 * grain

alpha = np.clip(d * env_v * env_h * grain * 1.45, 0.0, 0.58)

# A bed of pigment across the nav row only, so knocked-out white links always
# have contrast without flattening the clouds further down.
nav = smoothstep(0.32, 0.0, ys) ** 1.4 * smoothstep(0.15, 0.39, xw) * smoothstep(0.85, 0.61, xw)
alpha = np.clip(alpha + nav * (1.0 - alpha) * 0.62, 0.0, 0.84)

# Palette: a cool, slightly grey cerulean at the zenith easing to a hazy horizon
# blue. Keeping part of the source hue leaves the cloud shadows grey-violet.
zenith = np.array([0.16, 0.40, 0.60], dtype=np.float32)
horizon = np.array([0.50, 0.66, 0.77], dtype=np.float32)
t = smoothstep(0.0, 0.80, ys)[..., None]
pig = pig * 0.42 + (zenith * (1 - t) + horizon * t) * 0.58
# Keep every pigment in sky range: no channel may out-rank the one above it, so
# the paper flecks in the source can never read as brown specks.
pig[..., 1] = np.maximum(pig[..., 1], pig[..., 0])
pig[..., 2] = np.maximum(pig[..., 2], pig[..., 1])

# Drop the alpha noise floor to fully clear paper so empty regions compress.
# The threshold has to stay tiny here: the side falloff is so gradual that a
# harder cut would leave a visible contour down each margin.
alpha = np.where(alpha < 0.008, 0.0, alpha)

out = np.zeros((H, W, 4), dtype=np.uint8)
out[..., :3] = np.clip(pig * 255.0 + 0.5, 0, 255).astype(np.uint8)
out[..., 3] = np.clip(alpha * 255.0 + 0.5, 0, 255).astype(np.uint8)
Image.fromarray(out, "RGBA").save(OUT, quality=82, method=6)


def luminance(c):
    c = np.where(c <= 0.03928, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)
    return float(0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2])


# What the nav row actually sits on, composited over white.
for name, xf in (("brand 12%", 0.122), ("link 44%", 0.44), ("link 58%", 0.58), ("github 88%", 0.878)):
    rows = slice(int(0.04 * H), int(0.15 * H))
    cols = slice(max(0, int(xf * W) - 30), int(xf * W) + 30)
    a = float(alpha[rows, cols].mean())
    c = pig[rows, cols].reshape(-1, 3).mean(axis=0) * a + (1 - a)
    lw = (1.05) / (luminance(c) + 0.05)
    lk = (luminance(c) + 0.05) / (luminance(np.array([0.102, 0.114, 0.141])) + 0.05)
    hexc = "#%02x%02x%02x" % tuple(int(v * 255) for v in c)
    print(f"{name}: alpha={a:.2f} {hexc}  white-text {lw:.2f}:1  ink-text {lk:.2f}:1")
print("wrote", OUT)
