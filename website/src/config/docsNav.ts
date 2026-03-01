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
		label: "Usage",
		items: [
			{ href: "/docs/execution-modes/", label: "Execution Modes" },
			{ href: "/docs/repl-reference/", label: "REPL Reference" },
			{ href: "/docs/ast-and-bytecode-tools/", label: "AST and Bytecode Tools" },
			{ href: "/docs/reference/", label: "CLI and Environment" },
			{ href: "/docs/diagnostics/", label: "Diagnostics and Tracebacks" },
			{ href: "/docs/import-paths/", label: "Import and Stdlib Paths" },
			{ href: "/docs/troubleshooting/", label: "Troubleshooting" },
		],
	},
	{
		label: "Compatibility",
		items: [
			{ href: "/docs/compatibility/", label: "Compatibility Targets" },
			{ href: "/docs/stdlib-compatibility/", label: "Stdlib Compatibility" },
		],
	},
	{
		label: "Extensions",
		items: [{ href: "/docs/native-extensions/", label: "Native Extensions" }],
	},
];
