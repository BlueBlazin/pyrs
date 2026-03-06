export type ReferenceRow = {
	left: string;
	right: string;
};

export const invocationModeRows: ReferenceRow[] = [
	{
		left: "pyrs",
		right: "Start interactive REPL (or read from stdin when piped).",
	},
	{
		left: "pyrs path/to/script.py",
		right: "Run a Python source file.",
	},
	{
		left: "pyrs path/to/module.pyc",
		right: "Run a CPython bytecode file.",
	},
	{
		left: "pyrs -c \"code\"",
		right: "Execute inline source passed on the command line.",
	},
	{
		left: "pyrs -m package.module",
		right: "Resolve and run a library module as a script.",
	},
];

export const cliFlagRows: ReferenceRow[] = [
	{
		left: "-h, --help",
		right: "Print command-line usage help.",
	},
	{
		left: "-V, --version",
		right: "Print interpreter version.",
	},
	{
		left: "--ast path.py",
		right: "Print parsed AST for a source file.",
	},
	{
		left: "--bytecode path.py",
		right: "Print bytecode disassembly for a source file.",
	},
	{
		left: "-S, --no-site",
		right: "Disable automatic site import at startup.",
	},
	{
		left: "-m <module>",
		right: "Run a library module as a script using CPython-shaped module mode.",
	},
	{
		left: "-W option, -Woption",
		right: "Configure warning filters.",
	},
	{
		left: "-X no_debug_ranges",
		right: "Disable traceback caret range debug metadata.",
	},
	{
		left: "-X tracemalloc, -X tracemalloc=<N>",
		right: "Enable tracemalloc startup configuration.",
	},
	{
		left: "-I, -E, -u, -B",
		right: "Accepted CPython compatibility flags; currently no-op except that -I and -E suppress PYTHONWARNINGS ingest.",
	},
];

export const envVarRows: ReferenceRow[] = [
	{
		left: "PYRS_CPYTHON_LIB",
		right: "Set explicit CPython stdlib root; PYRS keeps stdlib imports isolated to that tree and borrows host lib-dynload only when needed.",
	},
	{
		left: "XDG_DATA_HOME",
		right: "Changes where installer-managed stdlib bundles are discovered (${XDG_DATA_HOME}/pyrs/stdlib/3.14.3/Lib).",
	},
	{
		left: "PYRS_REPL_THEME",
		right: "Set REPL theme mode: auto, dark, or light.",
	},
	{
		left: "PYTHONWARNINGS",
		right: "Startup warning filters consumed by warning configuration.",
	},
	{
		left: "PYTHONPATH",
		right: "Additional module search entries appended during startup path setup.",
	},
	{
		left: "PYTHONHOME",
		right: "Fallback CPython stdlib root when no managed or explicit stdlib root is selected.",
	},
	{
		left: "VIRTUAL_ENV",
		right: "Detected virtualenv site-packages entries are appended when present.",
	},
	{
		left: "COLORFGBG",
		right: "Used for auto light/dark terminal color tuning in CLI output and REPL theming.",
	},
];
