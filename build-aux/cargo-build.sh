#!/bin/sh
set -eu

source_root="$1"
build_root="$2"
output="$3"
profile="$4"
app_id="$5"
cargo_bin="$6"

export CARGO_TARGET_DIR="$build_root/target"
export BROOKLET_APP_ID="$app_id"

if [ "$profile" = "release" ]; then
  "$cargo_bin" build --manifest-path "$source_root/Cargo.toml" --locked --features gui --release
  install -m 0755 "$CARGO_TARGET_DIR/release/brooklet" "$output"
else
  "$cargo_bin" build --manifest-path "$source_root/Cargo.toml" --locked --features gui
  install -m 0755 "$CARGO_TARGET_DIR/debug/brooklet" "$output"
fi
