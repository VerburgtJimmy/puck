# puck site

Marketing site + plain docs for [puck](https://github.com/VerburgtJimmy/puck).
Astro 7, static output, deployed to **Cloudflare Pages** at
`https://puck.jimmyverburgt.com`.

## Local

```bash
cd site
npm install
npm run dev
```

`predev` / `prebuild` copy `../install.sh` → `public/install`.

```bash
npm run build
npm run preview
```

## Cloudflare Pages

| Setting | Value |
|---|---|
| Root directory | `site` |
| Build command | `npm run build` |
| Output directory | `dist` |
| Node | `22` |

Custom domain: `puck.jimmyverburgt.com` → Cloudflare DNS (CNAME to Pages).

`public/_headers` sets `Content-Type: text/plain` for `/install`.

## Docs

Plain pages under `/docs` (markdown-shaped Astro). No search engine yet.
A custom OSS docs package can land later without changing the domain.
