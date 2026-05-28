#!/bin/sh
# Dinopod install script — https://github.com/dinogomez/dinopod
#
# Usage:
#   curl -fsSL https://install.dinopod.dev | sh
#   curl -fsSL https://install.dinopod.dev/install.sh | sh
#
# Environment:
#   DINOPOD_VERSION      Pin a release tag (e.g. v0.1.0). Default: latest published release.
#   DINOPOD_INSTALL_DIR  Install directory. Default: ~/.local/bin
#
# Supported platforms (v1):
#   Linux x86_64 (glibc), macOS Intel, macOS Apple Silicon
#
# Upgrade: re-run this installer (idempotent overwrite).

set -euo pipefail

GITHUB_REPO="${DINOPOD_GITHUB_REPO:-dinogomez/dinopod}"
INSTALL_DIR="${DINOPOD_INSTALL_DIR:-${HOME}/.local/bin}"
VERSION="${DINOPOD_VERSION:-}"
DOWNLOAD_BASE="${DINOPOD_DOWNLOAD_BASE:-https://github.com/${GITHUB_REPO}/releases/download}"
LATEST_URL="${DINOPOD_LATEST_URL:-https://github.com/${GITHUB_REPO}/releases/latest}"

err() {
  printf 'dinopod install: %s\n' "$*" >&2
}

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    err "required command not found: $1"
    exit 1
  fi
}

detect_target() {
  os="$(uname -s)"
  arch="$(uname -m)"

  case "${os}" in
    Linux)
      if [ -f /etc/alpine-release ] || (command -v ldd >/dev/null 2>&1 && ldd /bin/sh 2>/dev/null | grep -q musl); then
        err "Alpine/musl Linux is not supported in v1 (glibc x86_64 only)"
        exit 2
      fi
      case "${arch}" in
        x86_64 | amd64) printf '%s' "x86_64-unknown-linux-gnu" ;;
        *)
          err "unsupported Linux architecture: ${arch}"
          exit 2
          ;;
      esac
      ;;
    Darwin)
      case "${arch}" in
        x86_64) printf '%s' "x86_64-apple-darwin" ;;
        arm64) printf '%s' "aarch64-apple-darwin" ;;
        *)
          err "unsupported macOS architecture: ${arch}"
          exit 2
          ;;
      esac
      ;;
    *)
      err "unsupported operating system: ${os} (use GitHub Releases for Windows)"
      exit 2
      ;;
  esac
}

resolve_version() {
  if [ -n "${VERSION}" ]; then
    printf '%s' "${VERSION}"
    return
  fi

  effective_url="$(curl -fsSL -o /dev/null -w '%{url_effective}' "${LATEST_URL}")"
  tag="${effective_url##*/}"
  if [ -z "${tag}" ] || [ "${tag}" = "latest" ]; then
    err "could not resolve latest published release"
    exit 3
  fi
  printf '%s' "${tag}"
}

sha256_file() {
  file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${file}" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "${file}" | awk '{print $1}'
  else
    err "required command not found: sha256sum or shasum"
    exit 1
  fi
}

verify_checksum() {
  artifact="$1"
  sidecar="$2"
  expected="$(awk '{print $1}' "${sidecar}")"
  actual="$(sha256_file "${artifact}")"
  if [ "${expected}" != "${actual}" ]; then
    err "checksum verification failed for ${artifact}"
    exit 4
  fi
}

path_contains_dir() {
  dir="$1"
  old_ifs="${IFS}"
  IFS=:
  for entry in ${PATH:-}; do
    if [ "${entry}" = "${dir}" ]; then
      IFS="${old_ifs}"
      return 0
    fi
  done
  IFS="${old_ifs}"
  return 1
}

main() {
  need_cmd curl
  need_cmd tar
  need_cmd mkdir
  need_cmd mv
  need_cmd chmod

  if [ "$(id -u)" -eq 0 ]; then
    err "running as root is discouraged; install to a user directory instead"
    err "set DINOPOD_INSTALL_DIR explicitly if you intend a system-wide install"
  fi

  target="$(detect_target)"
  version="$(resolve_version)"
  archive="dinopod-${version}-${target}.tar.gz"
  base_url="${DOWNLOAD_BASE}/${version}"

  tmpdir=""
  cleanup() {
    if [ -n "${tmpdir}" ] && [ -d "${tmpdir}" ]; then
      rm -rf "${tmpdir}"
    fi
  }
  trap cleanup EXIT INT TERM

  tmpdir="$(mktemp -d)"
  archive_path="${tmpdir}/${archive}"
  sidecar_path="${tmpdir}/${archive}.sha256"

  if ! curl -fsSL -o "${sidecar_path}" "${base_url}/${archive}.sha256"; then
    err "failed to download checksum for ${archive} (is ${version} published?)"
    exit 3
  fi

  if ! curl -fsSL -o "${archive_path}" "${base_url}/${archive}"; then
    err "failed to download ${archive}"
    exit 3
  fi

  verify_checksum "${archive_path}" "${sidecar_path}"

  extract_dir="${tmpdir}/extract"
  mkdir -p "${extract_dir}"
  tar -xzf "${archive_path}" -C "${extract_dir}"

  if [ ! -f "${extract_dir}/dinopod" ]; then
    err "archive did not contain dinopod binary"
    exit 4
  fi

  chmod +x "${extract_dir}/dinopod"
  mkdir -p "${INSTALL_DIR}"

  dest="${INSTALL_DIR}/dinopod"
  if ! mv "${extract_dir}/dinopod" "${dest}"; then
    err "failed to install to ${dest} (is dinopod currently running?)"
    exit 5
  fi

  printf 'dinopod %s installed to %s\n' "${version}" "${dest}"

  if ! path_contains_dir "${INSTALL_DIR}"; then
    printf 'Add %s to your PATH, for example:\n' "${INSTALL_DIR}"
    printf '  export PATH="%s:$PATH"\n' "${INSTALL_DIR}"
  fi
}

main "$@"
