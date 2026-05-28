#!/usr/bin/env bash

setup() {
  export BATS_TEST_TMPDIR="${BATS_TEST_TMPDIR:-${BATS_TMPDIR}}"
  INSTALL_ROOT="${BATS_TEST_DIRNAME}/../.."
  FIXTURE_VERSION="v0.1.0-test"
  FIXTURE_TARGET="x86_64-unknown-linux-gnu"
  FIXTURE_DIR="${BATS_TEST_DIRNAME}/fixtures/${FIXTURE_VERSION}"

  if [ ! -f "${FIXTURE_DIR}/dinopod-${FIXTURE_VERSION}-${FIXTURE_TARGET}.tar.gz" ]; then
    bash "${BATS_TEST_DIRNAME}/fixtures/build.sh"
  fi

  INSTALL_DIR="${BATS_TEST_TMPDIR}/install-bin"
  mkdir -p "${INSTALL_DIR}"
  export DINOPOD_INSTALL_DIR="${INSTALL_DIR}"
  export DINOPOD_VERSION="${FIXTURE_VERSION}"
  export DINOPOD_DOWNLOAD_BASE="file://${FIXTURE_DIR}"
  export PATH="${BATS_TEST_DIRNAME}/mocks:${PATH}"
  export MOCK_UNAME_S="Linux"
  export MOCK_UNAME_M="x86_64"
}

run_installer() {
  sh "${INSTALL_ROOT}/scripts/install.sh"
}
