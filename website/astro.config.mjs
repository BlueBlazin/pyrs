// @ts-check
import { defineConfig } from 'astro/config';
import mdx from '@astrojs/mdx';

// https://astro.build/config
const astroBase = process.env.ASTRO_BASE || '/';
const astroSite = process.env.ASTRO_SITE || 'https://blueblazin.github.io';

export default defineConfig({
	site: astroSite,
	base: astroBase,
	output: 'static',
	integrations: [mdx()],
});
