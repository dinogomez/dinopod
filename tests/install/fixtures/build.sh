#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
version="v0.1.0-test"
target="x86_64-unknown-linux-gnu"
archive="dinopod-${version}-${target}.tar.gz"
release_dir="${root}/${version}"
build_dir="${release_dir}/build"

rm -rf "${release_dir}"
mkdir -p "${build_dir}"

cat >"${build_dir}/dinopod" <<'EOF'
#!/bin/sh
echo "dinopod-test"
EOF
chmod +x "${build_dir}/dinopod"

tar -C "${build_dir}" -czf "${release_dir}/${archive}" dinopod

(
  cd "${release_dir}"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${archive}" >"${archive}.sha256"
  else
    shasum -a 256 "${archive}" >"${archive}.sha256"
  fi
)

echo "fixtures built in ${release_dir}"
