import type { APIRoute } from "astro";

const normalizeBase = (value: string) => {
	if (value === "/") {
		return "";
	}
	return value.endsWith("/") ? value.slice(0, -1) : value;
};

const withBase = (path: string) => `${normalizeBase(import.meta.env.BASE_URL)}${path}`;

export const GET: APIRoute = ({ site }) => {
	const origin = site ?? new URL(import.meta.env.SITE || "https://blueblazin.github.io");
	const sitemap = new URL(withBase("/sitemap.xml"), origin).toString();
	const body = `User-agent: *\nAllow: /\nSitemap: ${sitemap}\n`;

	return new Response(body, {
		headers: {
			"Content-Type": "text/plain; charset=utf-8",
		},
	});
};
