import typography from "@tailwindcss/typography";

/** @type {import('tailwindcss').Config} */
export default {
	content: ["./src/**/*.{astro,html,js,jsx,md,mdx,ts,tsx}"],
	theme: {
		extend: {
			colors: {
				docs: {
					bg: "#090b11",
					surface: "#0f131c",
					line: "#242936",
					text: "#f3f6fb",
					muted: "#a5aec0",
				},
			},
		},
	},
	plugins: [typography],
};
