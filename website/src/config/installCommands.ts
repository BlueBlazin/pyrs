export type TerminalLine = string | { text: string; prompt?: boolean };

export const githubInstallerCommands = [
	"curl -fsSL https://raw.githubusercontent.com/BlueBlazin/pyrs/master/scripts/install.sh | bash",
	"pyrs --version",
];

export const githubInstallerStableCommands = [
	"curl -fsSL https://raw.githubusercontent.com/BlueBlazin/pyrs/master/scripts/install.sh | bash -s -- --stable",
	"pyrs --version",
];

export const homebrewHeadCommands = ["brew install --HEAD blueblazin/tap/pyrs", "pyrs --version"];

export const cargoInstallGitHubCommands = [
	"cargo install --locked --git https://github.com/BlueBlazin/pyrs --bin pyrs",
	"pyrs --version",
];

export const cargoInstallRepoPathCommands = [
	"git clone https://github.com/BlueBlazin/pyrs.git",
	"cd pyrs",
	"cargo install --locked --path .",
	"pyrs --version",
];

export const cargoSourceBuildCommands = [
	"git clone https://github.com/BlueBlazin/pyrs.git",
	"cd pyrs",
	"cargo build --release",
	"target/release/pyrs --version",
];

export const cargoSourceBuildRunCommands = [
	"git clone https://github.com/BlueBlazin/pyrs.git",
	"cd pyrs",
	"cargo build --release",
	"target/release/pyrs",
];

export const dockerNightlyCommands = [
	"docker pull ghcr.io/blueblazin/pyrs:nightly",
	"docker run --rm -it ghcr.io/blueblazin/pyrs:nightly",
];

export const nightlyArchiveMacosCommands = [
	"ARCH=\"$(uname -m)\"",
	"case \"$ARCH\" in",
	"  arm64) PKG=\"pyrs-nightly-aarch64-apple-darwin.tar.gz\" ;;",
	"  *)     PKG=\"pyrs-nightly-x86_64-apple-darwin.tar.gz\" ;;",
	"esac",
	"curl -L \"https://github.com/BlueBlazin/pyrs/releases/download/nightly/$PKG\" -o \"$PKG\"",
	"tar -xzf \"$PKG\"",
	"./pyrs --version",
];

export const nightlyArchiveLinuxCommands = [
	"PKG=\"pyrs-nightly-x86_64-unknown-linux-gnu.tar.gz\"",
	"curl -L \"https://github.com/BlueBlazin/pyrs/releases/download/nightly/$PKG\" -o \"$PKG\"",
	"tar -xzf \"$PKG\"",
	"./pyrs --version",
];

export const quickRunCommands = ["pyrs", "pyrs -c \"import sys; print(sys.version)\"", "pyrs path/to/script.py"];

export const nightlyArchiveMacosTerminalCommands: TerminalLine[] = [
	"ARCH=\"$(uname -m)\"",
	"case \"$ARCH\" in",
	{ text: "  arm64) PKG=\"pyrs-nightly-aarch64-apple-darwin.tar.gz\" ;;", prompt: false },
	{ text: "  *)     PKG=\"pyrs-nightly-x86_64-apple-darwin.tar.gz\" ;;", prompt: false },
	{ text: "esac", prompt: false },
	"curl -L \"https://github.com/BlueBlazin/pyrs/releases/download/nightly/$PKG\" -o \"$PKG\"",
	"tar -xzf \"$PKG\"",
	"./pyrs --version",
];
