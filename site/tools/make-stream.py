"""Bake the second appearance of the river.

The page's one recurring motif is the water: it leaves the hero, crosses the
fold, and then shows up once more near the bottom before the footer's horizon.
This is the same painted river, lifted out of the field art by keeping only the
cool pigment -- grass, rocks and grass tufts drop out, so it reads as a stretch
of water on open paper rather than a second copy of the hero's scenery.
"""

import numpy as np
from PIL import Image

SRC = "/Users/jimmyverburgt/Developer/personal/puck/site/art-src/wash-field.png"
OUT = "/Users/jimmyverburgt/Developer/personal/puck/site/public/art/stream.webp"

# Lower-left of the sheet: the widest, loosest part of the river.
CROP = (120, 640, 1000, 1030)

src = Image.open(SRC).convert("RGB").crop(CROP)
W, H = src.size
rgb = np.asarray(src, dtype=np.float32) / 255.0

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


# Water only: blue has to out-rank green by a clear margin. Grass is yellow-green
# and the rocks are neutral, so both fall away.
cool = smoothstep(0.02, 0.16, pig[..., 2] - pig[..., 1])
alpha = ink * cool

# Feather every edge so the stretch of water sits on open paper.
alpha = alpha * smoothstep(0.0, 0.10, xs) * smoothstep(1.0, 0.88, xs)
alpha = alpha * smoothstep(0.0, 0.09, ys) * smoothstep(1.0, 0.86, ys)
alpha = np.where(alpha < 0.02, 0.0, alpha)

out = np.zeros((H, W, 4), dtype=np.uint8)
out[..., :3] = np.clip(pig * 255.0 + 0.5, 0, 255).astype(np.uint8)
out[..., 3] = np.clip(alpha * 255.0 + 0.5, 0, 255).astype(np.uint8)
Image.fromarray(out, "RGBA").save(OUT, quality=82, method=6)
print("wrote", OUT, (W, H))
