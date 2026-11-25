"""Run the sealed peers phase from a base that exposes the pinned stock inventories."""
import argparse
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import recovery as prior

original = prior.original
run = original.run
ID = original.ID + "-peer-stage-recovery"
OUT = HERE / "evidence"
PACKET = HERE / "controller-packet.json"
FAILED = prior.OUT / "peers"
PILOT = prior.OUT / "pilot"
SEAL = "3d14a97d7e937beaa7667445ace42d14954839993e1b01674139407bbcb4e714"


def remote(phase, label):
    assert phase == "peers"
    return run.REMOTE / (ID + "-peers") / original.LANES.get(label, label)


def inventory(phase, label):
    result = prior.inventory(phase, label)
    if label in original.ENGINES:
        return result
    old, new = map(str, (prior.remote(phase, label), remote(phase, label)))

    def replace(value):
        return new + value[len(old):] if value.startswith(old + "/") else value

    spec = result["engines"]["mesh"]
    spec["argv"] = [replace(arg) for arg in spec["argv"]]
    spec["verified_files"] = {replace(path): digest for path, digest in spec["verified_files"].items()}
    return result


def failed_receipt():
    receipt = FAILED / "controller-release.json"
    released = run.read_json(receipt)
    assert released["id"] == prior.ID + "-peers"
    assert released["request_sha256"] == SEAL and released["exit_status"] == 1
    assert released["grant"] == run.read_json(FAILED / "grant.json")
    log = (FAILED / "controller.log").read_text()
    assert "FileNotFoundError:" in log and f"/{original.ID}/path-recovery/peer-inventories/river.json" in log
    assert not (FAILED / "blocks").exists() and not list(FAILED.rglob("raw.tar.gz"))
    assert run.read_json(FAILED / "staging/A1/inventory.json") == prior.inventory("peers", "A1")
    return dict(receipt_sha256=run.digest(receipt), original_arms=0)


def packet():
    assert run.digest(prior.PACKET) == SEAL
    paths = [HERE / name for name in ("README.md", "peer_recovery.py", "test_staging.py")]
    paths.extend(HERE / "peer-inventories" / f"{label}.json" for label in original.ENGINES)
    paths.extend(path for path in FAILED.rglob("*") if path.is_file())
    paths.extend([prior.PACKET, PILOT / "controller-release.json", PILOT / "replay.json"])
    return dict(id=ID + "-peers", inherits_reservation=prior.ID + "-peers", no_build=True,
                public_elf_sha256=original.PUBLIC_ELF, original_failure=failed_receipt(),
                plan=original.packet()["plans"]["peers"], pins={
                    str(path.relative_to(original.ROOT)): run.digest(path) for path in sorted(paths)})


def check():
    qualification = prior.check()
    assert run.read_json(PACKET) == packet()
    pilot = prior.driver()["replay"]("pilot", qualification)
    assert json.loads(json.dumps(pilot)) == run.read_json(PILOT / "replay.json")
    assert pilot["advance_to_peers"]
    for label in original.ENGINES:
        path = HERE / "peer-inventories" / f"{label}.json"
        canonical = original.HERE / "peer-inventories" / f"{label}.json"
        assert path.resolve(strict=True) == canonical
        assert run.read_json(path) == inventory("peers", label)
    return qualification


def driver():
    scope = prior.driver()
    scope.update(HERE=HERE, OUT=OUT, ID=ID, remote=remote, inventory=inventory)
    replay = scope["replay"]

    def release_for(phase, qualification):
        if phase == "pilot":
            return prior.driver()["replay"]("pilot", qualification)
        return replay(phase, qualification)

    scope["replay"] = release_for
    return scope


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("seal", "check", "dispatch", "replay"))
    parser.add_argument("digest", nargs="?")
    parser.add_argument("--controller-root", type=Path)
    args = parser.parse_args()
    if args.command == "seal":
        run.write_json(PACKET, packet())
        return
    qualification = check()
    if args.command == "check":
        print(run.digest(PACKET))
    elif args.command == "replay":
        print(json.dumps(driver()["replay"]("peers", qualification), indent=2, sort_keys=True))
    else:
        driver()["dispatch"]("peers", args.digest, args.controller_root.resolve(strict=True), qualification)


if __name__ == "__main__":
    main()
