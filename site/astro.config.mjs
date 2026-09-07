// @ts-check
import { defineConfig } from 'astro/config';
import mdx from '@astrojs/mdx';

export default defineConfig({
  site: 'https://puck.jimmyverburgt.com',
  integrations: [mdx()],
  build: {
    inlineStylesheets: 'auto',
  },
});
