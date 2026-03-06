#!/usr/bin/env bash
set -euo pipefail

REPO="BlueBlazin/pyrs"
CPYTHON_STDLIB_VERSION="3.14.3"
BIN_DIR="${HOME}/.local/bin"
DATA_HOME_DEFAULT="${XDG_DATA_HOME:-${HOME}/.local/share}"
DATA_DIR="${DATA_HOME_DEFAULT}/pyrs"
TAG=""
USE_NIGHTLY=1
CHANNEL_FLAG_SET=0
UNINSTALL_ONLY=0
FORCE_BUNDLED_STDLIB=0

usage() {
  cat <<'USAGE'
Install pyrs from GitHub releases (binary + CPython 3.14.3 stdlib bundle when needed).

Usage:
  install.sh [--nightly] [--stable] [--tag <tag>] [--repo <owner/repo>] [--bin-dir <dir>] [--data-dir <dir>] [--force-bundled-stdlib]
  install.sh --uninstall [--bin-dir <dir>] [--data-dir <dir>]

Examples:
  curl -fsSL https://raw.githubusercontent.com/BlueBlazin/pyrs/master/scripts/install.sh | bash
  curl -fsSL https://raw.githubusercontent.com/BlueBlazin/pyrs/master/scripts/install.sh | bash -s -- --stable
  curl -fsSL https://raw.githubusercontent.com/BlueBlazin/pyrs/master/scripts/install.sh | bash -s -- --tag v0.4.2
  curl -fsSL https://raw.githubusercontent.com/BlueBlazin/pyrs/master/scripts/install.sh | bash -s -- --uninstall
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --nightly)
      USE_NIGHTLY=1
      CHANNEL_FLAG_SET=1
      shift
      ;;
    --stable)
      USE_NIGHTLY=0
      CHANNEL_FLAG_SET=1
      shift
      ;;
    --tag)
      TAG="$2"
      shift 2
      ;;
    --repo)
      REPO="$2"
      shift 2
      ;;
    --bin-dir)
      BIN_DIR="$2"
      shift 2
      ;;
    --data-dir)
      DATA_DIR="$2"
      shift 2
      ;;
    --uninstall)
      UNINSTALL_ONLY=1
      shift
      ;;
    --force-bundled-stdlib)
      FORCE_BUNDLED_STDLIB=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -n "${TAG}" && "${CHANNEL_FLAG_SET}" -eq 1 ]]; then
  echo "error: --nightly/--stable cannot be combined with --tag" >&2
  exit 2
fi

checksum_file_verify() {
  local checksum_file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c "${checksum_file}"
    return
  fi
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -c "${checksum_file}"
    return
  fi
  echo "error: need sha256sum or shasum for checksum verification" >&2
  exit 1
}

normalize_stdlib_root() {
  local candidate="$1"
  if [[ -z "${candidate}" || ! -f "${candidate}/site.py" ]]; then
    return 1
  fi
  (
    cd "${candidate}" >/dev/null 2>&1
    pwd -P
  ) || printf "%s" "${candidate}"
}

probe_python_stdlib_root() {
  local python_bin
  local root
  for python_bin in python3.14 python3; do
    if ! command -v "${python_bin}" >/dev/null 2>&1; then
      continue
    fi
    root="$("${python_bin}" -c 'import sys, sysconfig; print(sysconfig.get_path("stdlib") if sys.version_info[:2] == (3, 14) else "")' 2>/dev/null || true)"
    if [[ -n "${root}" ]] && normalize_stdlib_root "${root}" >/dev/null; then
      normalize_stdlib_root "${root}"
      return 0
    fi
  done
  return 1
}

detect_existing_cpython_stdlib_root() {
  local root
  if root="$(probe_python_stdlib_root)"; then
    printf "%s" "${root}"
    return 0
  fi
  if [[ -n "${PYTHONHOME:-}" ]] && normalize_stdlib_root "${PYTHONHOME}/lib/python3.14" >/dev/null; then
    normalize_stdlib_root "${PYTHONHOME}/lib/python3.14"
    return 0
  fi
  for root in \
    /Library/Frameworks/Python.framework/Versions/3.14/lib/python3.14 \
    /opt/homebrew/Frameworks/Python.framework/Versions/3.14/lib/python3.14 \
    /usr/local/lib/python3.14 \
    /usr/lib/python3.14; do
    if normalize_stdlib_root "${root}" >/dev/null; then
      normalize_stdlib_root "${root}"
      return 0
    fi
  done
  return 1
}

legacy_data_dir() {
  printf "%s/.local/share/pyrs" "${HOME}"
}

remove_if_empty_upwards() {
  local path="$1"
  while [[ -n "${path}" && "${path}" != "/" && "${path}" != "." ]]; do
    if ! rmdir "${path}" 2>/dev/null; then
      break
    fi
    path="$(dirname "${path}")"
  done
}

remove_managed_stdlib() {
  local data_dir="$1"
  local version_dir="${data_dir}/stdlib/${CPYTHON_STDLIB_VERSION}"
  if [[ -d "${version_dir}" ]]; then
    rm -rf "${version_dir}"
    remove_if_empty_upwards "${version_dir}"
  fi
}

uninstall_pyrs() {
  local removed=0
  local candidate_data_dirs=("${DATA_DIR}")
  local legacy_dir
  legacy_dir="$(legacy_data_dir)"
  if [[ "${legacy_dir}" != "${DATA_DIR}" ]]; then
    candidate_data_dirs+=("${legacy_dir}")
  fi

  if [[ -f "${BIN_DIR}/pyrs" ]]; then
    rm -f "${BIN_DIR}/pyrs"
    removed=1
  fi

  for data_dir in "${candidate_data_dirs[@]}"; do
    if [[ -d "${data_dir}/stdlib/${CPYTHON_STDLIB_VERSION}" ]]; then
      remove_managed_stdlib "${data_dir}"
      removed=1
    fi
  done

  if [[ "${removed}" -eq 0 ]]; then
    echo "Nothing to uninstall."
    return 0
  fi

  echo "Removed:"
  echo "  binary: ${BIN_DIR}/pyrs"
  for data_dir in "${candidate_data_dirs[@]}"; do
    echo "  stdlib: ${data_dir}/stdlib/${CPYTHON_STDLIB_VERSION}"
  done
}

detect_target_triple() {
  local os
  local arch
  case "$(uname -s)" in
    Linux) os="unknown-linux-gnu" ;;
    Darwin) os="apple-darwin" ;;
    *)
      echo "error: unsupported OS: $(uname -s)" >&2
      exit 1
      ;;
  esac
  case "$(uname -m)" in
    x86_64|amd64) arch="x86_64" ;;
    arm64|aarch64)
      if [[ "${os}" == "unknown-linux-gnu" ]]; then
        echo "error: native Linux arm64/aarch64 binaries are not part of the current release matrix" >&2
        exit 1
      fi
      arch="aarch64"
      ;;
    *)
      echo "error: unsupported architecture: $(uname -m)" >&2
      exit 1
      ;;
  esac
  printf "%s-%s" "${arch}" "${os}"
}

resolve_latest_tag() {
  local repo="$1"
  local tag
  tag="$(curl -fsSL "https://api.github.com/repos/${repo}/releases?per_page=1" \
    | sed -n 's/^[[:space:]]*"tag_name":[[:space:]]*"\([^"]*\)".*$/\1/p' \
    | head -n 1)"
  if [[ -z "${tag}" ]]; then
    echo "error: failed to resolve latest published release tag for ${repo}" >&2
    exit 1
  fi
  printf "%s" "${tag}"
}

download_asset() {
  local url="$1"
  local out="$2"
  curl -fsSL --retry 3 --retry-all-errors -o "${out}" "${url}"
}

if [[ "${UNINSTALL_ONLY}" -eq 1 ]]; then
  uninstall_pyrs
  exit 0
fi

if ! command -v curl >/dev/null 2>&1; then
  echo "error: curl is required" >&2
  exit 1
fi
if ! command -v tar >/dev/null 2>&1; then
  echo "error: tar is required" >&2
  exit 1
fi

TARGET_TRIPLE="$(detect_target_triple)"
if [[ -z "${TAG}" ]]; then
  if [[ "${USE_NIGHTLY}" -eq 1 ]]; then
    TAG="nightly"
  else
    TAG="$(resolve_latest_tag "${REPO}")"
  fi
fi

if [[ "${TAG}" == "nightly" ]]; then
  BINARY_ASSET="pyrs-nightly-${TARGET_TRIPLE}.tar.gz"
else
  BINARY_ASSET="pyrs-${TAG}-${TARGET_TRIPLE}.tar.gz"
fi
STDLIB_ASSET="pyrs-stdlib-cpython-${CPYTHON_STDLIB_VERSION}.tar.gz"
CHECKSUMS_ASSET="SHA256SUMS"
BASE_URL="https://github.com/${REPO}/releases/download/${TAG}"
HOST_STDLIB_ROOT=""
INSTALL_MANAGED_STDLIB=1
if [[ "${FORCE_BUNDLED_STDLIB}" -eq 0 ]] && HOST_STDLIB_ROOT="$(detect_existing_cpython_stdlib_root)"; then
  INSTALL_MANAGED_STDLIB=0
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

if [[ "${INSTALL_MANAGED_STDLIB}" -eq 1 ]]; then
  echo "Downloading ${BINARY_ASSET} and ${STDLIB_ASSET} from ${REPO}@${TAG}..."
else
  echo "Downloading ${BINARY_ASSET} from ${REPO}@${TAG}..."
  echo "Using existing CPython 3.14 stdlib at ${HOST_STDLIB_ROOT}; bundled Lib/ install will be skipped."
fi
download_asset "${BASE_URL}/${BINARY_ASSET}" "${TMP_DIR}/${BINARY_ASSET}"
if [[ "${INSTALL_MANAGED_STDLIB}" -eq 1 ]]; then
  download_asset "${BASE_URL}/${STDLIB_ASSET}" "${TMP_DIR}/${STDLIB_ASSET}"
fi
download_asset "${BASE_URL}/${CHECKSUMS_ASSET}" "${TMP_DIR}/${CHECKSUMS_ASSET}"

if ! grep -q "  ${BINARY_ASSET}\$" "${TMP_DIR}/${CHECKSUMS_ASSET}"; then
  echo "error: ${BINARY_ASSET} not found in ${CHECKSUMS_ASSET}" >&2
  exit 1
fi
if [[ "${INSTALL_MANAGED_STDLIB}" -eq 1 ]] && ! grep -q "  ${STDLIB_ASSET}\$" "${TMP_DIR}/${CHECKSUMS_ASSET}"; then
  echo "error: ${STDLIB_ASSET} not found in ${CHECKSUMS_ASSET}" >&2
  exit 1
fi
grep "  ${BINARY_ASSET}\$" "${TMP_DIR}/${CHECKSUMS_ASSET}" > "${TMP_DIR}/checksums.verify"
if [[ "${INSTALL_MANAGED_STDLIB}" -eq 1 ]]; then
  grep "  ${STDLIB_ASSET}\$" "${TMP_DIR}/${CHECKSUMS_ASSET}" >> "${TMP_DIR}/checksums.verify"
fi
(cd "${TMP_DIR}" && checksum_file_verify "checksums.verify")

mkdir -p "${BIN_DIR}"
tar -xzf "${TMP_DIR}/${BINARY_ASSET}" -C "${TMP_DIR}"
install -m 0755 "${TMP_DIR}/pyrs" "${BIN_DIR}/pyrs"

STDLIB_DEST="${DATA_DIR}/stdlib/${CPYTHON_STDLIB_VERSION}"
if [[ "${INSTALL_MANAGED_STDLIB}" -eq 1 ]]; then
  mkdir -p "${STDLIB_DEST}"
  tar -xzf "${TMP_DIR}/${STDLIB_ASSET}" -C "${TMP_DIR}"
  STDLIB_ROOT="${TMP_DIR}/pyrs-stdlib-cpython-${CPYTHON_STDLIB_VERSION}"
  if [[ ! -d "${STDLIB_ROOT}/Lib" ]]; then
    echo "error: stdlib bundle is missing Lib/ payload" >&2
    exit 1
  fi
  rm -rf "${STDLIB_DEST}/Lib"
  cp -R "${STDLIB_ROOT}/Lib" "${STDLIB_DEST}/Lib"
  if [[ -f "${STDLIB_ROOT}/LICENSE" ]]; then
    cp "${STDLIB_ROOT}/LICENSE" "${STDLIB_DEST}/LICENSE"
  fi
else
  remove_managed_stdlib "${DATA_DIR}"
  legacy_dir="$(legacy_data_dir)"
  if [[ "${legacy_dir}" != "${DATA_DIR}" ]]; then
    remove_managed_stdlib "${legacy_dir}"
  fi
fi

echo
echo "Installed:"
echo "  binary: ${BIN_DIR}/pyrs"
if [[ "${INSTALL_MANAGED_STDLIB}" -eq 1 ]]; then
  echo "  stdlib: ${STDLIB_DEST}/Lib"
else
  echo "  stdlib: using host CPython 3.14 at ${HOST_STDLIB_ROOT}"
fi
if [[ ":${PATH}:" != *":${BIN_DIR}:"* ]]; then
  echo
  echo "Add ${BIN_DIR} to your PATH to run 'pyrs' directly."
fi
echo "Run: ${BIN_DIR}/pyrs --version"
