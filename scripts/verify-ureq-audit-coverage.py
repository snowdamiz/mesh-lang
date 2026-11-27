#!/usr/bin/env python3
"""Bind the HTTP advisory audit to the local packages selected by Mesh."""

import json
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ORIGINALS = {
    "ureq": (
        "3.5.2",
        "3ac5cecf619ca81f66a96482db09687c9f7465af65f4300a8c30cca05bd5a59d",
        "51bb001154f44cfab69653a74f7d45fbe1995b22",
    ),
    "ureq-proto": (
        "0.7.1",
        "622dce33c252115466d83f63e9744a18f12b4406aae51a01c15ad8fcf1185eb3",
        "acb5cc3ee1261709316acd6538be08e82f84f3c6",
    ),
}


def read_toml(path):
    with (ROOT / path).open("rb") as stream:
        return tomllib.load(stream)


def require(condition, message):
    if not condition:
        raise SystemExit(message)


def verify():
    audited = read_toml("scripts/ureq-upstream-audit.lock")["package"]
    require(
        len(audited) == len(ORIGINALS) and {entry["name"] for entry in audited} == ORIGINALS.keys(),
        "audit must include both ureq crates",
    )
    for entry in audited:
        name = entry["name"]
        version, checksum, commit = ORIGINALS[name]
        require(
            entry["version"] == version
            and entry["checksum"] == checksum
            and entry["source"] == "registry+https://github.com/rust-lang/crates.io-index",
            f"{name}: audit must use the original registry identity",
        )
        directory = ROOT / "third_party" / name
        manifest = read_toml(directory / "Cargo.toml")["package"]
        vcs = json.loads((directory / ".cargo_vcs_info.json").read_text(encoding="utf-8"))
        require(
            manifest["name"] == name and manifest["version"] == version and vcs["git"]["sha1"] == commit,
            f"{name}: vendored source must match the audited release",
        )
        for lockfile in ("Cargo.lock", "fuzz/Cargo.lock"):
            selected = [item for item in read_toml(lockfile)["package"] if item["name"] == name]
            require(
                len(selected) == 1 and selected[0]["version"] == version and "source" not in selected[0],
                f"{lockfile}: expected exactly one local {name} {version}",
            )

    root_ureq = read_toml("Cargo.toml")["workspace"]["dependencies"]["ureq"]
    require(
        (ROOT / root_ureq["path"]).resolve() == ROOT / "third_party" / "ureq"
        and root_ureq["version"] == "=3.5.2",
        "workspace Cargo.toml must select the vendored ureq 3.5.2",
    )
    proto_dep = read_toml("third_party/ureq/Cargo.toml")["dependencies"]["ureq-proto"]
    require(
        (ROOT / "third_party" / "ureq" / proto_dep["path"]).resolve()
        == ROOT / "third_party" / "ureq-proto"
        and proto_dep["version"] == "=0.7.1",
        "vendored ureq must select the adjacent vendored ureq-proto 0.7.1",
    )

    print("ureq upstream audit coverage verified for workspace and fuzz locks")


if __name__ == "__main__":
    verify()
