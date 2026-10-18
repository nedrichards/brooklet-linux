#!/usr/bin/env bash
# Keep local source builds and checks in one Meson/Cargo tree in the pinned SDK.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
case "${1:-}" in
  compile|test) operation=$1; shift ;;
  -h|--help) echo "Usage: $0 {compile|test} [Meson arguments...]"; exit 0 ;;
  *) echo "Usage: $0 {compile|test} [Meson arguments...]" >&2; exit 2 ;;
esac
mapfile -t sdk_config < <(python3 - <<'PY'
import json
from pathlib import Path
manifest = json.loads(Path('flatpak/com.nedrichards.brooklet.Devel.json').read_text())
print(f"{manifest['sdk']}//{manifest['runtime-version']}")
print(manifest['build-options']['append-path'])
PY
)
mkdir -p "$root/_build/cargo-home"
exec flatpak run --user --share=network --command=sh \
  --filesystem="$root" \
  --env="CARGO_HOME=$root/_build/cargo-home" \
  --env="CARGO_TARGET_DIR=$root/_build/target" \
  "${sdk_config[0]}" -eu -c '
    export PATH="$1:$PATH"
    cd "$2"
    operation=$3
    shift 3
    if [ ! -f _build/build.ninja ]; then
      meson setup _build --buildtype=debugoptimized -Dprofile=development
    fi
    if [ "$operation" = test ]; then
      exec meson test -C _build --num-processes=1 --timeout-multiplier=3 --print-errorlogs "$@"
    fi
    exec meson compile -C _build "$@"
  ' sh "${sdk_config[1]}" "$root" "$operation" "$@"
