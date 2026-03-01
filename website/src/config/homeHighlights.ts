export type HomeHighlight = {
	title: string;
	summary: string;
	href: string;
	linkLabel: string;
};

export const homeHighlights: HomeHighlight[] = [
	{
		title: "Docs Shell Is Live",
		summary: "Getting Started, Installation, and Reference pages now share a consistent docs layout.",
		href: "/docs/",
		linkLabel: "Read docs",
	},
	{
		title: "Install Paths Unified",
		summary: "Home and docs now use shared install-command sources to prevent snippet drift.",
		href: "/docs/install/",
		linkLabel: "View install",
	},
	{
		title: "Reference + Authoring",
		summary: "CLI/env reference and an internal docs style guide are available for ongoing content work.",
		href: "/docs/style-guide/",
		linkLabel: "Open style guide",
	},
];
