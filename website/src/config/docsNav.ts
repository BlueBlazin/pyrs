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
			{ href: "/docs/playground/", label: "Browser Playground" },
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
			{ href: "/docs/language-compatibility/", label: "Language Compatibility" },
			{ href: "/docs/stdlib-compatibility/", label: "Stdlib Compatibility" },
			{ href: "/docs/scientific-stack-status/", label: "Scientific Stack Status" },
		],
	},
	{
		label: "Extensions",
		items: [
			{ href: "/docs/native-extensions/", label: "Native Extensions" },
			{ href: "/docs/extensions/capi-v1/", label: "Extension C-API v1" },
		],
	},
	{
		label: "Project",
		items: [{ href: "/docs/roadmap-and-milestones/", label: "Roadmap and Workstreams" }],
	},
	{
		label: "Contributing",
		items: [
			{ href: "/docs/contributing/architecture/", label: "Architecture" },
			{ href: "/docs/contributing/validation/", label: "Validation and Gates" },
		],
	},
];
