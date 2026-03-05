#!/usr/bin/env bash
set -euo pipefail

REPO="BlueBlazin/pyrs"
CPYTHON_STDLIB_VERSION="3.14.3"
BIN_DIR="${HOME}/.local/bin"
DATA_DIR="${HOME}/.local/share/pyrs"
TAG=""
USE_NIGHTLY=0

usage() {
  cat <<'USAGE'
Install pyrs from GitHub releases (binary + CPython 3.14.3 stdlib bundle).

Usage:
  install.sh [--nightly] [--tag <tag>] [--repo <owner/repo>] [--bin-dir <dir>] [--data-dir <dir>]

Examples:
  curl -fsSL https://raw.githubusercontent.com/BlueBlazin/pyrs/master/scripts/install.sh | bash
  curl -fsSL https://raw.githubusercontent.com/BlueBlazin/pyrs/master/scripts/install.sh | bash -s -- --nightly
  curl -fsSL https://raw.githubusercontent.com/BlueBlazin/pyrs/master/scripts/install.sh | bash -s -- --tag v0.3.0
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --nightly)
      USE_NIGHTLY=1
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

if [[ "${USE_NIGHTLY}" -eq 1 && -n "${TAG}" ]]; then
  echo "error: --nightly and --tag cannot be used together" >&2
  exit 2
fi

if ! command -v curl >/dev/null 2>&1; then
  echo "error: curl is required" >&2
  exit 1
fi
if ! command -v tar >/dev/null 2>&1; then
  echo "error: tar is required" >&2
  exit 1
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
    arm64|aarch64) arch="aarch64" ;;
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
  tag="$(curl -fsSL "https://api.github.com/repos/${repo}/releases/latest" \
    | sed -n 's/^[[:space:]]*"tag_name":[[:space:]]*"\([^"]*\)".*$/\1/p' \
    | head -n 1)"
  if [[ -z "${tag}" ]]; then
    echo "error: failed to resolve latest release tag for ${repo}" >&2
    exit 1
  fi
  printf "%s" "${tag}"
}

download_asset() {
  local url="$1"
  local out="$2"
  curl -fsSL --retry 3 --retry-all-errors -o "${out}" "${url}"
}

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

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

echo "Downloading ${BINARY_ASSET} and ${STDLIB_ASSET} from ${REPO}@${TAG}..."
download_asset "${BASE_URL}/${BINARY_ASSET}" "${TMP_DIR}/${BINARY_ASSET}"
download_asset "${BASE_URL}/${STDLIB_ASSET}" "${TMP_DIR}/${STDLIB_ASSET}"
download_asset "${BASE_URL}/${CHECKSUMS_ASSET}" "${TMP_DIR}/${CHECKSUMS_ASSET}"

if ! grep -q "  ${BINARY_ASSET}\$" "${TMP_DIR}/${CHECKSUMS_ASSET}"; then
  echo "error: ${BINARY_ASSET} not found in ${CHECKSUMS_ASSET}" >&2
  exit 1
fi
if ! grep -q "  ${STDLIB_ASSET}\$" "${TMP_DIR}/${CHECKSUMS_ASSET}"; then
  echo "error: ${STDLIB_ASSET} not found in ${CHECKSUMS_ASSET}" >&2
  exit 1
fi
grep "  ${BINARY_ASSET}\$" "${TMP_DIR}/${CHECKSUMS_ASSET}" > "${TMP_DIR}/checksums.verify"
grep "  ${STDLIB_ASSET}\$" "${TMP_DIR}/${CHECKSUMS_ASSET}" >> "${TMP_DIR}/checksums.verify"
(cd "${TMP_DIR}" && checksum_file_verify "checksums.verify")

mkdir -p "${BIN_DIR}"
tar -xzf "${TMP_DIR}/${BINARY_ASSET}" -C "${TMP_DIR}"
install -m 0755 "${TMP_DIR}/pyrs" "${BIN_DIR}/pyrs"

STDLIB_DEST="${DATA_DIR}/stdlib/${CPYTHON_STDLIB_VERSION}"
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

echo
echo "Installed:"
echo "  binary: ${BIN_DIR}/pyrs"
echo "  stdlib: ${STDLIB_DEST}/Lib"
if [[ ":${PATH}:" != *":${BIN_DIR}:"* ]]; then
  echo
  echo "Add ${BIN_DIR} to your PATH to run 'pyrs' directly."
fi
echo "Run: ${BIN_DIR}/pyrs --version"
