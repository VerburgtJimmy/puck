"""Bake the hero field art, split at the fold.

Same painting as art-src/wash-field.png, but the loose pink/violet streaks that
used to stand in for a sky are dropped -- the hero has a real sky band above it
now, and two skies in one frame read as a mistake.

The result is cut into two pieces at SPLIT: `field-scene` ends flush with the
bottom of the hero and `field-tail` opens the section below it. Both are placed
with the same width and right offset, so the river carries across the fold as
one continuous stroke instead of two that nearly line up.

Output is straight RGBA so the art floats on the page instead of sitting in a
white box.
"""

import numpy as np
from PIL import Image

SRC = "/Users/jimmyverburgt/Developer/personal/puck/site/art-src/wash-field.png"
OUT_DIR = "/Users/jimmyverburgt/Developer/personal/puck/site/public/art/"

CROP = (0, 150, 1385, 908)
HAZE_TOP, HAZE_END = 0.04, 0.56   # fraction of the cropped height
SPLIT = 620                        # row where the hero ends; river is mid-stroke here

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


alpha = ink.copy()

# Knock the backdrop streaks back to nothing. Drawn objects up there -- the lamp
# post and its bulb -- carry far more pigment than a wash does, so density
# protects them without cutting a rectangle out of the fade.
drawn = smoothstep(0.26, 0.48, ink)
# The lamp's halo is warm but thin, so density alone would erase it. It is the
# only warm thing on the right of the sheet, which is enough to single it out.
glow = np.clip((rgb[..., 0] - rgb[..., 2]) * 5.0, 0.0, 1.0) * smoothstep(0.60, 0.66, np.repeat(xs, H, axis=0))
drawn = np.maximum(drawn, glow)
haze = smoothstep(HAZE_TOP, HAZE_END, np.repeat(ys, W, axis=1))
alpha = alpha * np.maximum(haze, drawn)

# What haze survives should read as cool distance, not leftover sunset.
warm = smoothstep(0.30, 0.02, np.repeat(ys, W, axis=1))[..., None] * (1.0 - drawn[..., None])
cool = np.array([0.42, 0.56, 0.72], dtype=np.float32)
pig = pig * (1 - warm * 0.75) + cool * (warm * 0.75)

# Let the river dissolve off the very bottom instead of stopping at the edge.
# Only the tail gets this -- the scene has to butt up against the fold cleanly.
alpha = alpha * smoothstep(1.0, 0.86, np.repeat(ys, W, axis=1))

# Snap near-zero pigment to fully clear paper: invisible on a white page, and it
# lets the large empty regions compress instead of carrying alpha noise.
alpha = np.where(alpha < 0.035, 0.0, alpha)

out = np.zeros((H, W, 4), dtype=np.uint8)
out[..., :3] = np.clip(pig * 255.0 + 0.5, 0, 255).astype(np.uint8)
out[..., 3] = np.clip(alpha * 255.0 + 0.5, 0, 255).astype(np.uint8)
img = Image.fromarray(out, "RGBA")

for name, box in (("field-scene", (0, 0, W, SPLIT)), ("field-tail", (0, SPLIT, W, H))):
    piece = img.crop(box)
    piece.save(OUT_DIR + name + ".webp", quality=82, method=6)
    print("wrote", name, piece.size)
