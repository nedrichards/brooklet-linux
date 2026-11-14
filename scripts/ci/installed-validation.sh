#!/usr/bin/env bash
set -euo pipefail
app="$APP_ID"
args=(--user --socket=x11 --nosocket=wayland --env=GDK_BACKEND=x11 --env=GSK_RENDERER=cairo --env=GSETTINGS_BACKEND=memory --filesystem="$PWD:ro" --filesystem="$PWD/.ci-results")
flatpak run "${args[@]}" "$app" --pull-refresh-test
flatpak run "${args[@]}" "$app" --smoke-test
