import type { APIRoute } from "astro";

const routes = ["/", "/docs/", "/docs/install/", "/docs/reference/", "/docs/style-guide/"];

const normalizeBase = (value: string) => {
	if (value === "/") {
		return "";
	}
	return value.endsWith("/") ? value.slice(0, -1) : value;
};

const withBase = (path: string) => `${normalizeBase(import.meta.env.BASE_URL)}${path}`;

export const GET: APIRoute = ({ site }) => {
	const origin = site ?? new URL(import.meta.env.SITE || "https://blueblazin.github.io");
	const body = `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${routes
		.map((route) => `  <url><loc>${new URL(withBase(route), origin).toString()}</loc></url>`)
		.join("\n")}\n</urlset>\n`;

	return new Response(body, {
		headers: {
			"Content-Type": "application/xml; charset=utf-8",
		},
	});
};
