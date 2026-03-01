export type DocsNavItem = {
	href: string;
	label: string;
};

export type DocsNavGroup = {
	label: string;
	items: DocsNavItem[];
};

export const docsNavGroups: DocsNavGroup[] = [
	{
		label: "Start",
		items: [
			{ href: "/docs/", label: "Getting Started" },
			{ href: "/docs/install/", label: "Installation" },
			{ href: "/docs/repl/", label: "REPL and Execution" },
		],
	},
	{
		label: "Reference",
		items: [
			{ href: "/docs/reference/", label: "CLI and Environment" },
			{ href: "/docs/import-paths/", label: "Import and Stdlib Paths" },
			{ href: "/docs/compatibility/", label: "Compatibility Targets" },
		],
	},
];
