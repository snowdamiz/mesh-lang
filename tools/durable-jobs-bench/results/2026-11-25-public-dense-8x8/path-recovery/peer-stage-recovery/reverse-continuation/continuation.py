"""Keep the complete forward block and repeat only the interrupted reverse block."""
import argparse
from collections import Counter
import json
from pathlib import Path
import sys
import tarfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import peer_recovery as prior

original = prior.original
run = original.run
ID = original.ID + "-peer-reverse-continuation"
OUT = HERE / "evidence"
PACKET = HERE / "controller-packet.json"
PREVIOUS = prior.OUT / "peers"
ORIGINAL_SEAL = "f789f968a5c8efd8d0e847cee249b1f9f7f3d9520956f222d494dca0d5abbcd1"
FORWARD, REVERSE = original.TRIALS["peers"]


def remote(phase, label):
    assert phase == "peers"
    return run.REMOTE / (ID + "-peers") / original.LANES.get(label, label)


def inventory(phase, label):
    result = prior.inventory(phase, label)
    if label in original.ENGINES:
        return result
    previous = str(prior.remote(phase, label)) + "/"
    destination = str(remote(phase, label)) + "/"
    spec = result["engines"]["mesh"]
    spec["argv"] = [arg.replace(previous, destination, 1) if arg.startswith(previous) else arg
                    for arg in spec["argv"]]
    spec["verified_files"] = {
        path.replace(previous, destination, 1) if path.startswith(previous) else path: digest
        for path, digest in spec["verified_files"].items()
    }
    return result


def prior_release():
    path = PREVIOUS / "controller-release.json"
    receipt = run.read_json(path)
    assert receipt["id"] == prior.ID + "-peers"
    assert receipt["request_sha256"] == ORIGINAL_SEAL and receipt["exit_status"] == 1
    assert receipt["grant"] == run.read_json(PREVIOUS / "grant.json")
    assert "ValueError: build process overlapped arm" in (PREVIOUS / "controller.log").read_text()
    assert len(list(PREVIOUS.rglob("raw.tar.gz"))) == 19
    return run.digest(path)


def build_events(raw):
    with tarfile.open(raw) as archive, archive.extractfile("host-samples.jsonl") as stream:
        return [{"epoch_us": sample["epoch_us"], "builders": sample["builders"]}
                for line in stream if (sample := json.loads(line))["builders"]]


def retained_forward(qualification):
    prior_release()
    valid = PREVIOUS / "blocks" / "01-noop"
    invalid = PREVIOUS / "blocks" / "02-noop"
    score = prior.driver()["score"]
    assert run.read_json(valid / "order.json") == FORWARD
    assert run.read_json(invalid / "order.json") == REVERSE
    complete = {}
    for phase, label, folder in original.arms(valid, FORWARD):
        assert not build_events(folder / "raw.tar.gz")
        complete[f"{phase}-{label}"] = score(folder, phase, label, "peers", qualification)
    assert len(complete) == 12 and json.loads(json.dumps(complete)) == run.read_json(valid / "reconciled.json")

    observed = [(phase, label, folder) for phase, label, folder in original.arms(invalid, REVERSE)
                if (folder / "raw.tar.gz").exists()]
    assert len(observed) == 7
    assert [(phase, label) for phase, label, _ in observed] == [
        (phase, label) for phase, label, _ in list(original.arms(invalid, REVERSE))[:7]]
    for phase, label, folder in observed[:-1]:
        assert not build_events(folder / "raw.tar.gz")
        score(folder, phase, label, "peers", qualification)
    phase, label, contaminated = observed[-1]
    assert (phase, label) == ("warmup", "B")
    assert build_events(contaminated / "raw.tar.gz") == [
        {"epoch_us": 1795586648856821, "builders": [[3127312, "apt-get"]]}]
    try:
        score(contaminated, phase, label, "peers", qualification)
    except ValueError as exc:
        assert str(exc) == "build process overlapped arm"
    else:
        raise AssertionError("the contaminated reverse rotation passed")
    assert not (invalid / "reconciled.json").exists()
    return complete


def packet():
    assert run.digest(prior.PACKET) == ORIGINAL_SEAL
    paths = [HERE / name for name in ("README.md", "continuation.py", "test_continuation.py")]
    paths.extend(HERE / "peer-inventories" / f"{label}.json" for label in original.ENGINES)
    paths.append(prior.PACKET)
    paths.extend(path for path in PREVIOUS.rglob("*") if path.is_file())
    return dict(id=ID + "-peers", predecessor=prior.ID + "-peers", no_build=True,
                public_elf_sha256=original.PUBLIC_ELF, failed_release_sha256=prior_release(),
                grant="TIMING", minutes=12, schedule=[REVERSE], retained=[FORWARD], pins={
                    str(path.relative_to(original.ROOT)): run.digest(path) for path in sorted(paths)})


def check():
    qualification = prior.check()
    assert run.read_json(PACKET) == packet()
    retained = retained_forward(qualification)
    for label, engine in original.ENGINES.items():
        inherited = HERE / "peer-inventories" / f"{label}.json"
        canonical = original.HERE / "peer-inventories" / inherited.name
        assert inherited.resolve(strict=True) == canonical
        assert run.read_json(inherited) == inventory("peers", label)
        spec = inventory("peers", label)["engines"][engine]
        assert any(arg in spec["verified_files"] for arg in spec["argv"])
    return qualification, retained


def driver():
    scope = prior.driver()
    scope.update(HERE=HERE, OUT=OUT, ID=ID, remote=remote, inventory=inventory, TRIALS={"peers": [REVERSE]})
    return scope


def combine(forward, reverse):
    groups = {"1": forward, "2": reverse}
    measured = {}
    for trial in (FORWARD, REVERSE):
        group = str(trial["group"])
        assert set(groups[group]) == {f"{phase}-{label}" for label in trial["order"] for phase in trial["phases"]}
        measured[group] = {label: groups[group]["measured-" + label] for label in trial["order"]}
    assert len({arm["raw_sha256"] for rows in groups.values() for arm in rows.values()}) == 24
    matched = original.peer_comparison(measured)
    return dict(groups=groups, matched=matched, forward_source=prior.ID + "-peers", reverse_source=ID + "-peers")


def replay(qualification, retained):
    reverse = driver()["replay"]("peers", qualification)["groups"]
    assert set(reverse) == {"2"}
    return combine(retained, reverse["2"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("seal", "check", "dispatch", "replay"))
    parser.add_argument("digest", nargs="?")
    parser.add_argument("--controller-root", type=Path)
    args = parser.parse_args()
    if args.command == "seal":
        run.write_json(PACKET, packet())
        return
    qualification, retained = check()
    if args.command == "check":
        print(run.digest(PACKET))
    elif args.command == "replay":
        print(json.dumps(replay(qualification, retained), indent=2, sort_keys=True))
    else:
        driver()["dispatch"]("peers", args.digest, args.controller_root.resolve(strict=True), qualification)


if __name__ == "__main__":
    main()
