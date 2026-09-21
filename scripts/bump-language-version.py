#!/usr/bin/env python3
"""Compute and apply the Mesh language version for a release.

    bump-language-version.py next [major|minor|patch]
        Print the version after the newest `vX.Y.Z` tag, bumped as asked.
        With no tags yet, the first release is the current `meshc` version.

    bump-language-version.py set X.Y.Z
        Write X.Y.Z as the package version of every crate under compiler/.

The language version is the compiler's: every crate under compiler/ moves
together, and release.yml refuses a tag that does not match meshc and meshpkg.
The next version comes from the newest tag rather than from Cargo.toml, so a
branch whose manifests were never bumped still releases the right number.
Cargo.lock is not touched here; run `cargo update --workspace` afterwards.
"""

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SEMVER = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")


def parse(version):
    match = SEMVER.match(version)
    if not match:
        sys.exit(f"not a plain X.Y.Z version: {version!r}")
    return tuple(int(part) for part in match.groups())


def compiler_manifests():
    manifests = sorted(ROOT.glob("compiler/*/Cargo.toml"))
    if not manifests:
        sys.exit("no compiler crates found")
    return manifests


def package_version(manifest):
    in_package = False
    for line in manifest.read_text().splitlines():
        stripped = line.strip()
        if stripped.startswith("["):
            in_package = stripped == "[package]"
        elif in_package:
            match = re.match(r'version\s*=\s*"([^"]+)"', stripped)
            if match:
                return match.group(1)
    sys.exit(f"{manifest} has no [package] version")


def newest_tag():
    tags = subprocess.run(
        ["git", "tag", "--list", "v*"],
        cwd=ROOT, check=True, capture_output=True, text=True,
    ).stdout.split()
    versions = [parse(tag[1:]) for tag in tags if SEMVER.match(tag[1:])]
    return max(versions) if versions else None


def next_version(bump):
    latest = newest_tag()
    if latest is None:
        return package_version(ROOT / "compiler/meshc/Cargo.toml")
    major, minor, patch = latest
    if bump == "major":
        return f"{major + 1}.0.0"
    if bump == "minor":
        return f"{major}.{minor + 1}.0"
    return f"{major}.{minor}.{patch + 1}"


def set_version(version):
    parse(version)
    for manifest in compiler_manifests():
        lines = manifest.read_text().splitlines(keepends=True)
        in_package = False
        replaced = False
        for index, line in enumerate(lines):
            stripped = line.strip()
            if stripped.startswith("["):
                in_package = stripped == "[package]"
            elif in_package and not replaced and re.match(r"version\s*=", stripped):
                # Only the [package] version: dependency tables also carry
                # `version =`, and rewriting those would change what resolves.
                lines[index] = re.sub(r'"[^"]*"', f'"{version}"', line, count=1)
                replaced = True
        if not replaced:
            sys.exit(f"{manifest} has no [package] version to set")
        manifest.write_text("".join(lines))


def main(argv):
    if len(argv) >= 1 and argv[0] == "next":
        bump = argv[1] if len(argv) > 1 else "patch"
        if bump not in ("major", "minor", "patch"):
            sys.exit(f"bump must be major, minor or patch, not {bump!r}")
        print(next_version(bump))
    elif len(argv) == 2 and argv[0] == "set":
        set_version(argv[1])
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
