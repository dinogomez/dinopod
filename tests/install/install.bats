#!/usr/bin/env bats

load helpers/setup

@test "installs dinopod binary from fixture release" {
  run_installer

  [ "$status" -eq 0 ]
  [ -x "${DINOPOD_INSTALL_DIR}/dinopod" ]
  run "${DINOPOD_INSTALL_DIR}/dinopod"
  [ "$status" -eq 0 ]
  [[ "$output" == *"dinopod-test"* ]]
}

@test "reinstall overwrites existing binary" {
  run_installer
  [ "$status" -eq 0 ]

  run_installer
  [ "$status" -eq 0 ]
  [ -x "${DINOPOD_INSTALL_DIR}/dinopod" ]
}

@test "aborts when checksum does not match" {
  archive="dinopod-${FIXTURE_VERSION}-${FIXTURE_TARGET}.tar.gz"
  sidecar="${FIXTURE_DIR}/${archive}.sha256"
  backup="${BATS_TEST_TMPDIR}/sidecar.bak"

  cp "${sidecar}" "${backup}"
  printf '0000000000000000000000000000000000000000000000000000000000000000  %s\n' "${archive}" >"${sidecar}"

  run run_installer
  [ "$status" -eq 4 ]

  mv "${backup}" "${sidecar}"
  [ ! -x "${DINOPOD_INSTALL_DIR}/dinopod" ]
}

@test "fails on unsupported operating system" {
  export MOCK_UNAME_S="FreeBSD"
  export MOCK_UNAME_M="amd64"

  run run_installer
  [ "$status" -eq 2 ]
  [[ "$output" == *"unsupported operating system"* ]]
}

@test "fails when install directory is not writable" {
  readonly_dir="${BATS_TEST_TMPDIR}/readonly-bin"
  mkdir -p "${readonly_dir}"
  chmod 555 "${readonly_dir}"
  export DINOPOD_INSTALL_DIR="${readonly_dir}"

  run run_installer
  [ "$status" -ne 0 ]
}

@test "selects aarch64-apple-darwin artifact name on macOS arm64" {
  export MOCK_UNAME_S="Darwin"
  export MOCK_UNAME_M="arm64"

  arm64_target="aarch64-apple-darwin"
  arm64_archive="dinopod-${FIXTURE_VERSION}-${arm64_target}.tar.gz"
  cp "${FIXTURE_DIR}/dinopod-${FIXTURE_VERSION}-${FIXTURE_TARGET}.tar.gz" \
    "${FIXTURE_DIR}/${arm64_archive}"
  cp "${FIXTURE_DIR}/dinopod-${FIXTURE_VERSION}-${FIXTURE_TARGET}.tar.gz.sha256" \
    "${FIXTURE_DIR}/${arm64_archive}.sha256"
  sed "s/${FIXTURE_TARGET}/${arm64_target}/g" \
    "${FIXTURE_DIR}/${arm64_archive}.sha256" >"${BATS_TEST_TMPDIR}/arm64.sha256"
  mv "${BATS_TEST_TMPDIR}/arm64.sha256" "${FIXTURE_DIR}/${arm64_archive}.sha256"

  run run_installer
  [ "$status" -eq 0 ]
  [ -x "${DINOPOD_INSTALL_DIR}/dinopod" ]
}
