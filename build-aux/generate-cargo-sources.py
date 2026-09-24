#!/usr/bin/env python3
"""Regenerate Flatpak's offline Cargo sources from the checked-in lockfile."""

import json
import pathlib
import tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent
packages = tomllib.loads((ROOT / "Cargo.lock").read_text())["package"]
sources = []

for package in sorted(packages, key=lambda value: (value["name"], value["version"])):
    if package.get("source") != "registry+https://github.com/rust-lang/crates.io-index":
        continue
    name = package["name"]
    version = package["version"]
    checksum = package["checksum"]
    destination = f"cargo/vendor/{name}-{version}"
    sources.append({
        "type": "archive",
        "archive-type": "tar-gzip",
        "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
        "sha256": checksum,
        "dest": destination,
    })
    sources.append({
        "type": "inline",
        "contents": json.dumps({"package": checksum, "files": {}}, separators=(", ", ": ")),
        "dest": destination,
        "dest-filename": ".cargo-checksum.json",
    })

sources.append({
    "type": "inline",
    "contents": "[source.crates-io]\nreplace-with = \"vendored-sources\"\n\n[source.vendored-sources]\ndirectory = \"/app/share/brooklet/cargo/vendor\"\n",
    "dest": "cargo",
    "dest-filename": "config",
})

(ROOT / "flatpak/cargo-sources.json").write_text(json.dumps(sources, indent=4) + "\n")
