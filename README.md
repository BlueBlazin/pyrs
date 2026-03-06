<p align="center">
  <img src="website/public/images/pyrs-logo.png" alt="PYRS logo" width="124" />
</p>

<h1 align="center">PYRS</h1>

<p align="center"><strong>A Python interpreter in Rust targeting CPython 3.14 semantics.</strong></p>

<p align="center">
  <a href="#quick-start">Quick Start</a>
  ·
  <a href="#installation">Installation</a>
  ·
  <a href="#usage">Usage</a>
  ·
  <a href="#status">Status</a>
  ·
  <a href="#docs">Docs</a>
  ·
  <a href="#contributing">Contributing</a>
</p>

<p align="center">
  <img src="website/public/images/repl/repl1.png" alt="PYRS REPL screenshot 1" width="92%" />
</p>

## At a Glance

| Area | Current State |
| --- | --- |
| Compatibility target | CPython 3.14 |
| Core execution paths | Source (`.py`), bytecode (`.pyc`), interactive REPL |
| Platform priority | macOS + Linux (`x86_64`, `aarch64`) |
| C-extension support | In progress (scientific stack bring-up underway) |
| Local test runner | `cargo nextest run` |

<p align="center">
  <img src="website/public/images/repl/repl2.png" alt="PYRS REPL screenshot 2" width="92%" />
</p>

## Quick Start

Install from GitHub Releases (binary + bundled CPython 3.14.3 stdlib). Default channel is nightly:

```bash
curl -fsSL https://raw.githubusercontent.com/BlueBlazin/pyrs/master/scripts/install.sh | bash
pyrs --version
```

The installer writes `pyrs` to `~/.local/bin`. If a usable local CPython 3.14 stdlib is not already present, it also stages the official `Lib/` under `${XDG_DATA_HOME:-~/.local/share}/pyrs/stdlib/3.14.3/Lib`. When Python 3.14 is already installed, the installer skips the bundled stdlib and reuses the host stdlib instead.

Run interactive REPL:

```bash
pyrs
```

Run inline Python:

```bash
pyrs -c "import platform; print(platform.python_version())"
```

Run a script:

```bash
pyrs path/to/script.py
```

## Installation

### GitHub one-command installer (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/BlueBlazin/pyrs/master/scripts/install.sh | bash
```

Stable tagged channel:

```bash
curl -fsSL https://raw.githubusercontent.com/BlueBlazin/pyrs/master/scripts/install.sh | bash -s -- --stable
```

Uninstall:

```bash
curl -fsSL https://raw.githubusercontent.com/BlueBlazin/pyrs/master/scripts/install.sh | bash -s -- --uninstall
```

### Cargo install (bring your own CPython stdlib)

```bash
cargo install --locked --git https://github.com/BlueBlazin/pyrs --bin pyrs
```

`cargo install` only installs the `pyrs` binary. For full stdlib behavior, ensure CPython 3.14 stdlib is available (or set `PYRS_CPYTHON_LIB=/path/to/python3.14/Lib`).

### Cargo install from local repo path

```bash
git clone https://github.com/BlueBlazin/pyrs.git
cd pyrs
cargo install --locked --path .
```

### Build from source (no install)

```bash
git clone https://github.com/BlueBlazin/pyrs.git
cd pyrs
cargo build --release
./target/release/pyrs --version
```

### Homebrew (tap)

```bash
brew install --HEAD blueblazin/tap/pyrs
```

Uninstall with:

```bash
brew uninstall pyrs
```

### Docker nightly

```bash
docker pull ghcr.io/blueblazin/pyrs:nightly
docker run --rm -it ghcr.io/blueblazin/pyrs:nightly
```

### Nightly binary archives (advanced)

Nightly binary archives are published at:

- [GitHub Releases (nightly tag)](https://github.com/BlueBlazin/pyrs/releases/tag/nightly)

These archives are binary-only. Use them when CPython 3.14 is already available locally, or when you plan to place the separate `pyrs-stdlib-cpython-3.14.3.tar.gz` bundle yourself. If you want stdlib placement handled automatically, use the GitHub installer or Homebrew.

## Usage

### CLI execution modes

```bash
pyrs                         # REPL (or stdin when piped)
pyrs path/to/script.py       # source file
pyrs path/to/module.pyc      # CPython bytecode file
pyrs -c "print('hello')"     # inline source
```

### Useful flags

```bash
pyrs --help
pyrs --version
pyrs -S path/to/script.py
pyrs --ast path/to/script.py
pyrs --bytecode path/to/script.py
```

### REPL shortcuts

```text
:help   :clear   :paste   :timing   :reset   :quit
```

### Environment knobs

```text
PYRS_CPYTHON_LIB   explicit CPython stdlib root
PYRS_REPL_THEME    auto | dark | light
PYTHONPATH         additional import search paths
```

## Status

PYRS is an active pre-release project with CPython 3.14 parity as the correctness target.

### What works today

- Broad pure-Python runtime surface (modules/packages, classes, comprehensions, generators, core async/threading flows).
- CPython bytecode execution for supported `.pyc` surfaces.
- Interactive REPL with multiline input, history, syntax highlighting, and command utilities.
- Large and growing stdlib support coverage.

### In progress

- Long-tail CPython parity closure across stdlib/runtime edge behavior.
- Broader C-extension compatibility closure (NumPy/scientific-stack parity work).
- Additional performance and hardening milestones.

## Testing

Run the local suite with `nextest`:

```bash
cargo nextest run
```

Use `cargo test` only when you specifically need `cargo test` semantics:

```bash
cargo test
```

## Docs

- Website/docs source: [`website/`](website/)
- Project roadmap: [`docs/ROADMAP.md`](docs/ROADMAP.md)
- Compatibility tracker: [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md)
- Production readiness tracker: [`docs/PRODUCTION_READINESS.md`](docs/PRODUCTION_READINESS.md)
- Stub/partial parity ledger: [`docs/STUB_ACCOUNTING.md`](docs/STUB_ACCOUNTING.md)

## Contributing

PRs and focused bug reports are welcome.

- For CPython parity mismatches, include a minimal reproducer and both CPython + PYRS output.
- For implementation workflow expectations in this repo, see [`AGENTS.md`](AGENTS.md).
