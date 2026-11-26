"""Read sealed controller releases and export the successful phase bindings."""
import importlib.util
import json
from pathlib import Path
import sys
import tarfile
import tempfile

from pack import HERE, OLD, digest

archived = importlib.util.spec_from_file_location("original_rotation", OLD / "rotation.py")
original_rotation = importlib.util.module_from_spec(archived)
sys.modules[archived.name] = original_rotation
archived.loader.exec_module(original_rotation)
from rotation import load, replay

PACKET = HERE / "controller-packet.json"


def release(phase, folder):
    record = load(folder / "controller-release.json")
    expected = load(PACKET)["phases"][phase]["id"]
    assert (record["id"], record["exit_status"], record["request_sha256"]) == (expected, 0, digest(PACKET))
    raw = folder / "raw.tar.gz"
    assert record["raw_sha256"] == digest(raw)
    return record, raw


def extract(raw, folder):
    with tarfile.open(raw) as archive:
        archive.extractall(folder, filter="data")


def qualification(folder, temp):
    released, raw = release("qualification", folder)
    extract(raw, temp)
    summary = load(temp / "qualification.json")
    packet = load(PACKET)
    assert summary["passed"] is True and summary["capsule_sha256"] == packet["capsule_sha256"]
    for name, expected in summary["files"].items():
        assert digest(temp / name) == expected, name
    pins = load(temp / "build-pins.json")
    assert {lane: row["store_sha256"] for lane, row in pins.items()} == packet["store_sha256"]
    assert len({row["binary_sha256"] for row in pins.values()}) == 3
    wire = load(temp / "wire.json")
    assert wire["set_based_calls"] >= 6 and wire["postgres"].startswith("17.11")
    for i in range(3):
        assert wire[f"retry-plain-{i}"]["jobs"] == wire[f"retry-escaped-{i}"]["jobs"] == 1
        assert wire[f"lost-before-commit-{i}"]["at_fault"]["jobs"] == 0
        assert wire[f"lost-after-commit-{i}"]["at_fault"]["jobs"] == 1
    return {"qualification_sha256":digest(temp / "qualification.json"),
            "build_pins_sha256":digest(temp / "build-pins.json"),
            "semantic_sha256":digest(temp / "semantic.json"), "suffix":released["suffix"]}


def screen(folder, temp, qualification_binding):
    released, raw = release("screen", folder)
    assert released["suffix"] == qualification_binding["suffix"]
    assert released["qualification_sha256"] == qualification_binding["qualification_sha256"]
    extract(raw, temp)
    report = replay(temp)
    assert report == load(temp / "summary.json")
    for key in ("semantic_sha256", "build_pins_sha256"):
        assert report[key] == qualification_binding[key]
    return dict(qualification_binding, screen_sha256=digest(temp / "summary.json"),
                screen_raw_sha256=digest(raw), earned=report["profiles"]["plain"]["earned"],
                report=report)


if __name__ == "__main__":
    # The controller may run replay in its checkout. Keep transient unpacking there.
    action, qualified, *later = sys.argv[1:]
    with tempfile.TemporaryDirectory(dir=HERE) as work:
        bind = qualification(Path(qualified), Path(work) / "qualification")
        if action == "screen":
            bind = screen(Path(later[0]), Path(work) / "screen", bind)
        else:
            assert action == "qualification" and not later
    print(json.dumps(bind, sort_keys=True, indent=2, allow_nan=False))
