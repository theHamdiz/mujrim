#!/usr/bin/env bash
# Assemble dist/linux-x86_64: UI + engine + updater + engines + NNUE metadata.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="${ROOT}/dist/linux-x86_64"
RELEASE="${ROOT}/target/release"

log() { printf '==> %s\n' "$*"; }

log "Building release binaries"
cargo build --release -p mujrim-ui -p mujrim -p mujrim-updater -p mujrim-benchmarker -p mujrim-tooling

rm -rf "${DIST}"
mkdir -p "${DIST}"

copy_bin() {
  local name="$1"
  if [[ -x "${RELEASE}/${name}" ]]; then
    cp -f "${RELEASE}/${name}" "${DIST}/${name}"
    chmod +x "${DIST}/${name}"
    printf '  %s\n' "${name}"
  else
    printf '  skip missing %s\n' "${name}"
  fi
}

copy_bin mujrim-ui
copy_bin mujrim
copy_bin mujrim-updater
copy_bin mujrim-benchmarker
copy_bin mujrim-tooling

if [[ -d "${ROOT}/engines" ]]; then
  mkdir -p "${DIST}/engines"
  cp -a "${ROOT}/engines/." "${DIST}/engines/"
  mkdir -p "${RELEASE}/engines"
  cp -a "${ROOT}/engines/." "${RELEASE}/engines/"
  printf '  engines/\n'
fi

if [[ -d "${ROOT}/dist/nnue" ]]; then
  cp -a "${ROOT}/dist/nnue" "${DIST}/nnue"
elif [[ -d "${ROOT}/nnue" ]]; then
  cp -a "${ROOT}/nnue" "${DIST}/nnue"
fi

if [[ -d "${ROOT}/dist/books" ]]; then
  cp -a "${ROOT}/dist/books" "${DIST}/books"
elif [[ -d "${ROOT}/books" ]]; then
  cp -a "${ROOT}/books" "${DIST}/books"
fi

# Product adapters when already built beside the release dir.
MUJRIM_BIN="${DIST}/engines/mujrim/bin/linux-x86_64"
mkdir -p "${MUJRIM_BIN}"
for name in mujrim-elite mujrim-external mujrim-v60 mujrim-ak; do
  if [[ -x "${RELEASE}/${name}" ]]; then
    cp -f "${RELEASE}/${name}" "${MUJRIM_BIN}/${name}"
    chmod +x "${MUJRIM_BIN}/${name}"
  fi
done
if [[ -x "${DIST}/mujrim" && ! -e "${MUJRIM_BIN}/mujrim-external" ]]; then
  cp -f "${DIST}/mujrim" "${MUJRIM_BIN}/mujrim-external"
  chmod +x "${MUJRIM_BIN}/mujrim-external"
fi

log "dist/linux-x86_64 assembled"
find "${DIST}" -maxdepth 2 -printf '%p\n' | sed "s|${ROOT}/||" | head -n 40
