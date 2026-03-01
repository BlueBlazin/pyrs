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
		],
	},
	{
		label: "Reference",
		items: [{ href: "/docs/reference/", label: "CLI and Environment" }],
	},
];
