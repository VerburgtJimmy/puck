## Development

When starting the dev server, use background mode:

```
astro dev --background
```

Manage the background server with `astro dev stop`, `astro dev status`, and `astro dev logs`.

## Documentation

Full documentation: https://docs.astro.build

Consult these guides before working on related tasks:

- [Adding pages, dynamic routes, or middleware](https://docs.astro.build/en/guides/routing/)
- [Working with Astro components](https://docs.astro.build/en/basics/astro-components/)
- [Using React, Vue, Svelte, or other framework components](https://docs.astro.build/en/guides/framework-components/)
- [Adding or managing content](https://docs.astro.build/en/guides/content-collections/)
- [Adding styles or using Tailwind](https://docs.astro.build/en/guides/styling/)
- [Supporting multiple languages](https://docs.astro.build/en/guides/internationalization/)

## Hero art

The watercolor pieces on the home page are baked from the source paintings in
`art-src/` (not served) into `public/art/` by the scripts in `tools/`:

```
python3 tools/make-sky.py      # sky-band.webp   - hero sky, prints nav contrast
python3 tools/make-field.py    # field-scene.webp + field-tail.webp
python3 tools/make-stream.py   # stream.webp     - the water in the footer
```

`field-scene` ends flush with the bottom of the hero and `field-tail` opens the
section below it, both laid out with the same width and right offset, so the
river carries across the fold as one stroke. Changing `SPLIT` in
`make-field.py` moves that cut and needs both pieces re-baked together.

`node tools/check.mjs <url>` asserts the layout across ten widths: the install
command never overflows its pill, the page never scrolls sideways, and the hero
field and the river tail stay locked together. Run it after touching
`--art-right` / `--art-width` in `tokens.css`, which both halves of the river
share.

For visual checks, `node tools/shot.mjs <url> <out.png> [w] [h]` takes a
full-page screenshot over CDP and scrolls first so lazy art and the benchmark
bars have fired. Chrome's `--screenshot` flag only captures the viewport, which
is useless for a hero that is `100svh` tall.
