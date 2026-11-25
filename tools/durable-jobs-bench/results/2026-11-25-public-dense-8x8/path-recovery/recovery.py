"""Rebase the sealed canonical controller onto the released two-level layout."""
import argparse
import json
from pathlib import Path
import runpy
import sys
import tarfile

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent
sys.path.insert(0, str(PARENT))
import controller as original

run = original.run
ID = original.ID + "-path-recovery"
OUT = HERE / "evidence"
PACKET = HERE / "controller-packet.json"
FAILED = original.OUT / "activation"
ORIGINAL_SEAL = "581b7af161978a9728bb85cf028a19ba2f1afa3aeae395314d274277d9a70c12"
CASES = {
    "original": "lstat denied",
    "released_layout": "private write succeeded",
    "recovery_activation": "private write succeeded",
    "recovery_pilot": "private write succeeded",
    "blocked_traversal": "lstat denied",
}


def remote(phase, label):
    return run.REMOTE / f"{ID}-{phase}" / original.LANES.get(label, label)


def inventory(phase, label):
    result = original.inventory(phase, label)
    if label in original.ENGINES:
        return result
    old, new = map(str, (original.remote(phase, label), remote(phase, label)))

    def replace(value):
        return new + value[len(old):] if value.startswith(old + "/") else value

    spec = result["engines"]["mesh"]
    spec["argv"] = [replace(arg) for arg in spec["argv"]]
    spec["verified_files"] = {replace(path): digest for path, digest in spec["verified_files"].items()}
    return result


def failed_receipt():
    path = FAILED / "controller-release.json"
    release = run.read_json(path)
    assert release["id"] == original.ID + "-activation"
    assert release["request_sha256"] == ORIGINAL_SEAL and release["exit_status"] == 1
    assert release["grant"] == run.read_json(FAILED / "grant.json")
    raw, = FAILED.rglob("raw.tar.gz")
    folder = raw.parent
    attempt = run.read_json(folder / "request.json")["attempt"]
    assert run.read_json(folder / "dispatch.json")["exit_code"] == 1
    with tarfile.open(raw) as archive:
        assert run.file_json(archive, "invocation.json")["attempt"] == attempt
        assert run.file_json(archive, "status.json")["exit_code"] == 1
        log = archive.extractfile("adapter.log").read().decode()
    assert "out.is_symlink()" in log and "self.lstat()" in log
    assert "PermissionError: [Errno 13]" in log
    assert f"{original.remote('activation', 'A1')}/raw/{attempt}/adapter" in log
    return dict(id=release["id"], release_sha256=run.digest(path), raw_sha256=run.digest(raw),
                original_attempt=attempt)


def packet():
    paths = [HERE / name for name in ("README.md", "recovery.py", "prove_custody.py", "custody.json")]
    paths.append(PARENT / "controller-packet.json")
    paths.extend(path for path in FAILED.rglob("*") if path.is_file())
    return dict(id=ID, inherits_reservation=original.ID, no_build=True, keeper=original.TRANSFER,
                public_elf_sha256=original.PUBLIC_ELF, original_failure=failed_receipt(),
                plans=original.packet()["plans"], pins={
                    str(path.relative_to(original.ROOT)): run.digest(path) for path in sorted(paths)})


def check():
    assert run.digest(PARENT / "controller-packet.json") == ORIGINAL_SEAL
    qualified = original.check()
    assert run.read_json(PACKET) == packet()
    proof = run.read_json(HERE / "custody.json")
    assert proof["system"] == "Linux" and proof["runner"] == [65534, 65534]
    assert {case: value["outcome"] for case, value in proof["cases"].items()} == CASES
    assert proof["worker_sha256"] == run.digest(original.SUITE / "worker.py")
    assert proof["launcher_sha256"] == run.digest(PARENT / "stage/public-native-run")
    for phase in original.TRIALS:
        for trial in original.TRIALS[phase]:
            for label in trial["order"]:
                staged = remote(phase, label)
                assert staged.parent.parent == run.REMOTE
                config = inventory(phase, label)
                run.check_inventory(config)
                if label in original.LANES:
                    pins = config["engines"]["mesh"]["verified_files"]
                    assert all(pins[str(staged / path.name)] == run.digest(path) for path in original.staged(label))
                else:
                    assert config == original.inventory(phase, label)
    return qualified


def driver():
    source = runpy.run_path(str(PARENT / "controller.py"))
    scope = source["dispatch"].__globals__
    scope.update(HERE=HERE, OUT=OUT, ID=ID, remote=remote, inventory=inventory, staged=original.staged)
    return scope


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("seal", "check", "dispatch", "replay"))
    parser.add_argument("phase", choices=tuple(original.TRIALS), nargs="?")
    parser.add_argument("digest", nargs="?")
    parser.add_argument("--controller-root", type=Path)
    args = parser.parse_args()
    if args.command == "seal":
        run.write_json(PACKET, packet())
        return
    qualified = check()
    if args.command == "check":
        print(run.digest(PACKET))
    elif args.command == "replay":
        print(json.dumps(driver()["replay"](args.phase, qualified), indent=2, sort_keys=True))
    else:
        driver()["dispatch"](args.phase, args.digest, args.controller_root.resolve(strict=True), qualified)


if __name__ == "__main__":
    main()
