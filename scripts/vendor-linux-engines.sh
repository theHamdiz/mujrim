#!/usr/bin/env bash
# Vendor Linux x86_64 UCI engines into engines/<id>/bin/linux-x86_64/.
# Layout matches mujrim_protocols::catalog.
#
# Official GitHub release binaries are preferred. Engines without a Linux
# asset are built from the tagged source (Ethereal v13 last-free, lc0 CPU,
# Obsidian, Integral).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CACHE="${ROOT}/target/engine-vendor-cache"
ENGINES="${ROOT}/engines"
ARCH="linux-x86_64"
UA="mujrim-engine-vendor"
FAILED=()

mkdir -p "${CACHE}" "${ENGINES}"

log() { printf '==> %s\n' "$*"; }
warn() { printf '  warning: %s\n' "$*" >&2; }

download() {
  local url="$1" dest="$2"
  if [[ -f "${dest}" && -s "${dest}" ]]; then
    printf '  cache hit %s\n' "$(basename "${dest}")"
    return 0
  fi
  printf '  downloading %s\n' "${url}"
  curl -fL --retry 3 --retry-delay 2 -A "${UA}" -o "${dest}.partial" "${url}"
  mv "${dest}.partial" "${dest}"
}

place_engine() {
  local id="$1" src="$2"
  local dest_dir="${ENGINES}/${id}/bin/${ARCH}"
  mkdir -p "${dest_dir}"
  local dest="${dest_dir}/${id}"
  cp -f "${src}" "${dest}"
  chmod +x "${dest}"
  if command -v file >/dev/null 2>&1; then
    local info
    info="$(file -b "${dest}")"
    printf '  placed %s (%s)\n' "${dest#"${ROOT}/"}" "${info}"
    if ! grep -qiE 'ELF.*(x86-64|x86_64|64-bit)' <<<"${info}"; then
      warn "${id} is not an ELF x86-64 binary: ${info}"
      FAILED+=("${id}:arch")
    fi
  else
    printf '  placed %s\n' "${dest#"${ROOT}/"}"
  fi
}

extract_named() {
  local archive="$1" pattern="$2"
  local extract="${CACHE}/$(basename "${archive}")_extract"
  rm -rf "${extract}"
  mkdir -p "${extract}"
  case "${archive}" in
    *.tar|*.tar.gz|*.tgz) tar -xf "${archive}" -C "${extract}" ;;
    *.zip) unzip -qo "${archive}" -d "${extract}" ;;
    *) printf '%s\n' "${archive}"; return 0 ;;
  esac
  local found
  found="$(find "${extract}" -type f \( -name "${pattern}" -o -name "${pattern}*" \) ! -name '*.exe' | head -n 1 || true)"
  if [[ -z "${found}" ]]; then
    found="$(find "${extract}" -type f -executable ! -name '*.exe' | head -n 1 || true)"
  fi
  if [[ -z "${found}" ]]; then
    echo "no matching file for ${pattern} in ${archive}" >&2
    return 1
  fi
  printf '%s\n' "${found}"
}

uci_smoke() {
  local bin="$1"
  shift
  local extra=("$@")
  if [[ ! -x "${bin}" ]]; then
    warn "smoke skipped, missing ${bin}"
    FAILED+=("$(basename "${bin}"):missing")
    return 0
  fi
  local out
  if ! out="$(printf 'uci\nquit\n' | timeout 20s "${bin}" "${extra[@]}" 2>/dev/null || true)"; then
    out=""
  fi
  if grep -q 'uciok' <<<"${out}"; then
    printf '  uciok %s\n' "${bin#"${ROOT}/"}"
  else
    warn "no uciok from ${bin#"${ROOT}/"}"
    FAILED+=("$(basename "${bin}"):uciok")
  fi
}

vendor_url() {
  local id="$1" url="$2" cache_name="$3" pattern="${4:-}"
  log "Vendoring ${id}"
  local cache_path="${CACHE}/${cache_name}"
  if ! download "${url}" "${cache_path}"; then
    warn "${id} download failed"
    FAILED+=("${id}:download")
    return 0
  fi
  local src="${cache_path}"
  case "${cache_path}" in
    *.tar|*.tar.gz|*.tgz|*.zip)
      if ! src="$(extract_named "${cache_path}" "${pattern:-${id}}")"; then
        warn "${id} extract failed"
        FAILED+=("${id}:extract")
        return 0
      fi
      ;;
  esac
  place_engine "${id}" "${src}"
}

build_from_git() {
  local id="$1" repo="$2" tag="$3"
  shift 3
  local src_dir="${CACHE}/${id}-src"
  log "Building ${id} from ${repo}@${tag}"
  if [[ ! -d "${src_dir}/.git" ]]; then
    rm -rf "${src_dir}"
    git clone --depth 1 --branch "${tag}" --recurse-submodules --shallow-submodules "${repo}" "${src_dir}"
  fi
  (
    cd "${src_dir}"
    "$@"
  )
}

# ── Official Linux binaries ──────────────────────────────────────────────

vendor_url stockfish \
  "https://github.com/official-stockfish/Stockfish/releases/download/sf_18/stockfish-ubuntu-x86-64-avx2.tar" \
  "stockfish-ubuntu-x86-64-avx2-sf18.tar" \
  "stockfish"

vendor_url akimbo \
  "https://github.com/jw1912/akimbo/releases/download/v1.0.0/akimbo-1.0.0-avx2" \
  "akimbo-1.0.0-avx2"

vendor_url reckless \
  "https://github.com/codedeliveryservice/Reckless/releases/download/v0.9.0/reckless-linux-avx2" \
  "reckless-linux-avx2-v0.9.0"

vendor_url viridithas \
  "https://github.com/cosmobobak/viridithas/releases/download/v20.0.0/viridithas-20-linux-x86-64-v3" \
  "viridithas-20-linux-x86-64-v3"

vendor_url hobbes \
  "https://github.com/kelseyde/hobbes-chess-engine/releases/download/3.0/hobbes-linux-avx2" \
  "hobbes-linux-avx2-3.0"

vendor_url velvet \
  "https://github.com/mhonert/velvet-chess/releases/download/v8.1.1/velvet-v8.1.1-x86_64-avx2" \
  "velvet-v8.1.1-x86_64-avx2"

# ── Source builds when no Linux asset exists ─────────────────────────────

log "Building Ethereal v13 (last free GitHub release)"
if build_from_git ethereal "https://github.com/AndyGrant/Ethereal.git" "v13.00" \
  bash -lc 'make -C src -j"$(nproc)" EXE=ethereal 2>/dev/null || make -C src -j"$(nproc)"'; then
  eth_bin="$(find "${CACHE}/ethereal-src" -type f -name 'ethereal' -executable | head -n 1 || true)"
  if [[ -n "${eth_bin}" ]]; then
    place_engine ethereal "${eth_bin}"
  else
    warn "Ethereal built but binary not found"
    FAILED+=("ethereal:binary")
  fi
else
  warn "Ethereal source build failed"
  FAILED+=("ethereal:build")
fi

log "Building Obsidian v16 from tagged source"
if build_from_git obsidian "https://github.com/gab8192/Obsidian.git" "v16.0" \
  bash -lc 'make -j"$(nproc)" EXE=obsidian 2>/dev/null || make -j"$(nproc)"'; then
  obs_bin="$(find "${CACHE}/obsidian-src" -maxdepth 2 -type f -name 'obsidian' -executable | head -n 1 || true)"
  if [[ -z "${obs_bin}" ]]; then
    obs_bin="$(find "${CACHE}/obsidian-src" -maxdepth 2 -type f -executable ! -name '*.sh' ! -path '*/.git/*' | head -n 1 || true)"
  fi
  if [[ -n "${obs_bin}" ]]; then
    place_engine obsidian "${obs_bin}"
  else
    warn "Obsidian built but binary not found"
    FAILED+=("obsidian:binary")
  fi
else
  warn "Obsidian source build failed"
  FAILED+=("obsidian:build")
fi

log "Building Integral v7 from tagged source"
if build_from_git integral "https://github.com/aronpetko/integral.git" "v7" \
  bash -lc 'rm -rf build; make -j"$(nproc)" avx2 CXX=g++ CC=gcc'; then
  int_bin="$(find "${CACHE}/integral-src" -maxdepth 3 -type f \( -name 'integral' -o -name 'integral_*' \) ! -name '*.exe' -executable | head -n 1 || true)"
  if [[ -n "${int_bin}" ]]; then
    place_engine integral "${int_bin}"
  else
    warn "Integral built but binary not found"
    FAILED+=("integral:binary")
  fi
else
  warn "Integral source build failed"
  FAILED+=("integral:build")
fi

log "Building lc0 v0.32.1 CPU from tagged source"
LC0_OK=0
python3 -m venv "${CACHE}/meson-venv" >/dev/null 2>&1 || true
"${CACHE}/meson-venv/bin/pip" install -q meson >/dev/null 2>&1 || true
export PATH="${CACHE}/meson-venv/bin:${HOME}/.local/bin:${PATH}"
if build_from_git lc0 "https://github.com/LeelaChessZero/lc0.git" "v0.32.1" \
  bash -lc 'rm -rf build; ./build.sh -Dopencl=false -Dplain_cuda=false -Dcudnn=false -Donnx=false -Dispc=false -Dgtest=false'; then
  lc0_bin="$(find "${CACHE}/lc0-src/build" -type f -name 'lc0' -executable 2>/dev/null | head -n 1 || true)"
  if [[ -n "${lc0_bin}" ]]; then
    place_engine lc0 "${lc0_bin}"
    LC0_OK=1
  fi
fi
if [[ "${LC0_OK}" -ne 1 ]]; then
  warn "lc0 source build failed"
  FAILED+=("lc0:build")
fi

NET_URL="https://storage.lczero.org/files/networks-contrib/t1-256x10-distilled-swa-2432500.pb.gz"
NET_CACHE="${CACHE}/t1-256x10-distilled-swa-2432500.pb.gz"
if download "${NET_URL}" "${NET_CACHE}"; then
  mkdir -p "${ENGINES}/lc0/bin/${ARCH}"
  cp -f "${NET_CACHE}" "${ENGINES}/lc0/bin/${ARCH}/weights.pb.gz"
  printf '  placed lc0 weights %s\n' "engines/lc0/bin/${ARCH}/weights.pb.gz"
else
  warn "lc0 t1-256 net download failed"
  FAILED+=("lc0:net")
fi

# ── UCI smoke ────────────────────────────────────────────────────────────

log "UCI smoke"
for id in stockfish akimbo reckless viridithas hobbes velvet ethereal obsidian integral; do
  uci_smoke "${ENGINES}/${id}/bin/${ARCH}/${id}"
done
if [[ -f "${ENGINES}/lc0/bin/${ARCH}/weights.pb.gz" ]]; then
  uci_smoke "${ENGINES}/lc0/bin/${ARCH}/lc0" "--weights=${ENGINES}/lc0/bin/${ARCH}/weights.pb.gz"
elif [[ -x "${ENGINES}/lc0/bin/${ARCH}/lc0" ]]; then
  uci_smoke "${ENGINES}/lc0/bin/${ARCH}/lc0"
fi

# ── Mirror beside cargo run / dist ───────────────────────────────────────

sync_tree() {
  local dest="$1"
  mkdir -p "${dest}"
  if command -v rsync >/dev/null 2>&1; then
    rsync -a --delete "${ENGINES}/" "${dest}/"
  else
    cp -a "${ENGINES}/." "${dest}/"
  fi
  printf '  synced engines -> %s\n' "${dest#"${ROOT}/"}"
}

log "Syncing engine trees"
mkdir -p "${ROOT}/target/release" "${ROOT}/dist/linux-x86_64"
sync_tree "${ROOT}/target/release/engines"
sync_tree "${ROOT}/dist/linux-x86_64/engines"

log "Engine vendor complete"
find "${ENGINES}" -type f -path "*/bin/${ARCH}/*" -printf '%s %p\n' \
  | awk '{ printf "  %8.1f MB  %s\n", $1/1048576, $2 }' \
  | sed "s|${ROOT}/||"

if ((${#FAILED[@]})); then
  warn "failures: ${FAILED[*]}"
  exit 1
fi
