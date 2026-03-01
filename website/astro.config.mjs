// @ts-check
import { defineConfig } from 'astro/config';

// https://astro.build/config
const astroBase = process.env.ASTRO_BASE || '/';
const astroSite = process.env.ASTRO_SITE || 'https://blueblazin.github.io';

export default defineConfig({
	site: astroSite,
	base: astroBase,
	output: 'static',
});
