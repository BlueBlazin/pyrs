export type HomeHighlight = {
	title: string;
	summary: string;
	href: string;
	linkLabel: string;
};

export const homeHighlights: HomeHighlight[] = [
	{
		title: "Installation Paths",
		summary: "GitHub installer, Homebrew --HEAD, Cargo, Docker, and archive paths are documented with stdlib placement and uninstall details.",
		href: "/docs/install/",
		linkLabel: "Open install docs",
	},
	{
		title: "Execution Modes",
		summary: "REPL, stdin, -c, -m, source, and .pyc execution are documented with current sys.argv and startup semantics.",
		href: "/docs/execution-modes/",
		linkLabel: "Review execution",
	},
	{
		title: "Browser Playground",
		summary: "The website playground runs the wasm probe build in a worker and preloads a curated stdlib subset for quick experiments.",
		href: "/docs/playground/",
		linkLabel: "See playground docs",
	},
];
