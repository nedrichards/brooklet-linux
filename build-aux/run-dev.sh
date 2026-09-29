#!/usr/bin/env bash
# Run the local build against the installed development app's existing profile.
set -euo pipefail
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
app_profile="$HOME/.var/app/com.nedrichards.brooklet.Devel"
exec flatpak build \
  --socket=wayland --socket=fallback-x11 --share=ipc --share=network --device=dri \
  --talk-name=org.freedesktop.secrets --talk-name=org.freedesktop.portal.Flatpak \
  --filesystem="$repo_root:ro" --filesystem="$app_profile" \
  --env="XDG_DATA_HOME=$app_profile/data" \
  --env="XDG_CONFIG_HOME=$app_profile/config" \
  --env="XDG_CACHE_HOME=$app_profile/cache" \
  "$repo_root/build-dir" "$repo_root/_build/brooklet" "$@"
