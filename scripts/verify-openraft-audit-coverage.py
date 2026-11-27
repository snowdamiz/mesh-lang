#!/usr/bin/env python3
"""Bind the additional upstream audit to the vendored dependency versions."""

import json
import sys
import tomllib
from pathlib import Path


def load(path):
    with path.open("rb") as stream:
        return tomllib.load(stream)


def require(condition, message):
    if not condition:
        raise SystemExit(message)


def verify(root):
    upstream = load(root / "scripts/openraft-upstream-audit.lock").get("package", [])
    originals = {
        "openraft": ("80d35e2f60cdf9bcfc39a020966091017c6dc2a4b43b355a22ca3e76106f4a0a", "openraft"),
        "openraft-macros": ("bbbf0342d747a8da209c8e1d3ca8f788100966669412aaacb449409205931251", "macros"),
    }
    require(len(upstream) == 2 and {p["name"] for p in upstream} == originals.keys(),
            "upstream audit must include both OpenRaft crates")
    versions = {}
    for package in upstream:
        name = package["name"]
        require(package["source"] == "registry+https://github.com/rust-lang/crates.io-index",
                f"{name}: upstream audit source must be crates.io")
        require(package["version"] == "0.9.24" and package["checksum"] == originals[name][0],
                f"{name}: upstream audit must use the original published crate identity")
        versions[name] = package["version"]

    for workspace in (root, root / "fuzz"):
        manifest = load(workspace / "Cargo.toml")
        patches = manifest.get("patch", {}).get("crates-io", {})
        lock = load(workspace / "Cargo.lock")["package"]
        for name, version in versions.items():
            patch = patches.get(name, {})
            require("path" in patch, f"{workspace.name}: {name} must use the vendored patch")
            path = (workspace / patch["path"]).resolve()
            require(path == root / "third_party" / name,
                    f"{name}: vendored patch must use the recorded source directory")
            vendored = load(path / "Cargo.toml")["package"]
            require(vendored["name"] == name and vendored["version"] == version,
                    f"{name}: vendor manifest does not match upstream audit identity")
            vcs = json.loads((path / ".cargo_vcs_info.json").read_text(encoding="utf-8"))
            require(vcs["git"]["sha1"] == "da1c293692632005f82d180833a9085e3c192669"
                    and vcs["path_in_vcs"] == originals[name][1],
                    f"{name}: vendor provenance does not match the original 0.9.24 crate")
            entries = [p for p in lock if p["name"] == name]
            require(len(entries) == 1 and entries[0]["version"] == version and "source" not in entries[0],
                    f"{workspace.name}: {name} lock must contain exactly the audited vendored version")

    print("OpenRaft upstream audit coverage verified for workspace and fuzz locks")


if __name__ == "__main__":
    if len(sys.argv) != 1:
        raise SystemExit("usage: verify-openraft-audit-coverage.py")
    verify(Path(__file__).resolve().parents[1])
