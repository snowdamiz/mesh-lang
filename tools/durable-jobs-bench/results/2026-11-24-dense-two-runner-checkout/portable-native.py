"""Capture immutable native receipts and replay them without Linode or a checkout."""
import argparse
import gzip
import hashlib
import importlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import sys
import tarfile
import tempfile


REL = Path("tools/durable-jobs-bench/results/2026-11-24-dense-two-runner-checkout")
SUITE = REL.parents[1]
PREVIOUS = SUITE / "results/2026-11-23-dense-matched-topology"
D2 = SUITE / "results/2026-11-24-exit-confirmed-bulk-d2/recovery/evidence"
QUEUE = Path("target/linode-jobs-stepchange/requests")
ORIGINAL = REL / "evidence/native"
RETRY = REL / "evidence/native-import-retry"
ID = REL.name
RETRY_ID = ID + "-import-retry"
EXPECTED = {
    "original_series": "3b1611e742d60caaba65ca86bb86d8d46354ed200d34d8c2511ba9908f947fd3",
    "original_failed_arm": "cb31d68460ad2d66e4b4b48459d8198a2ad3fb0a86eba137d6f6d853b1f7bc5f",
    "original_request": "5519eab2776b8dc853f9db7ac60a46ead2278db492b6f7ba2848b53060f4dde9",
    "original_release": "03d14cba5dab5dded6fbc8591e72a93a10841b181b6be2063f647d47f02a6da6",
    "retry_request": "545f2843310221b6775dd6f5e67fd84c87eba5fe47d57c0e567541b4c1aea18a",
}
SCORE_SOURCE = [
    *(SUITE / name for name in ("run.py", "worker.py", "verify.py", "interleave.py")),
    *(PREVIOUS / name for name in ("matrix.py", "qualification.py", "equal.py", "native-mesh.sh")),
    *(Path("scripts") / name for name in ("jobs-parity.py", "jobs-parity-profiles.json")),
]
ARM_SOURCE = ("native-controller.py", "native-plan.json", "native-request.json", "native-score.py",
              "native-arm.py", "identity.py", "native-dense.sh", "bind-recovered-setup.py",
              "request-import-retry.json", "native-dense-import-retry.sh", "import-retry.py")


def check(test, message):
    if not test:
        raise ValueError(message)


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read(path):
    return json.loads(path.read_text())


def sources(root):
    evidence = root / REL / "evidence"
    yield from SCORE_SOURCE
    yield from (REL / name for name in ARM_SOURCE)
    yield from (REL / "evidence/import-proof" / name for name in ("probe.py", "linux.json"))
    yield from (QUEUE / name for name in (
        f"{ID}.json", f"{ID}-groups-1-2.released.json", f"{RETRY_ID}.json",
        "2026-11-24-exit-confirmed-bulk-d2-recovery.json",
        "2026-11-24-exit-confirmed-bulk-d2-recovery.released.json"))
    yield from (p.relative_to(root) for p in sorted((root / QUEUE).glob(RETRY_ID + "*.released.json")))
    yield D2 / "qualification.json"
    recovered = D2 / read(root / D2 / "qualification.json")["attempt"]
    yield from (recovered / name for name in ("status.json", "qualified-timing.json", "raw.tar.gz"))
    for folder in (evidence / "native", evidence / "native-import-retry"):
        for path in sorted(folder.rglob("*")):
            if path.is_file() and path.suffix in {".json", ".txt", ".log", ".gz"}:
                yield path.relative_to(root)


def pack(root, output):
    paths = list(sources(root))
    check(len(set(paths)) == len(paths), "duplicate paths when capturing")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=output.parent, prefix=output.name + ".", delete=False) as part:
        temp = Path(part.name)
        try:
            with gzip.GzipFile(filename="", fileobj=part, mode="wb", mtime=0, compresslevel=1) as gz:
                with tarfile.open(fileobj=gz, mode="w") as tar:
                    hashes = {}
                    for relative in sorted(paths):
                        path = root / relative
                        check(path.is_file() and not path.is_symlink(), f"missing safe source {relative}")
                        # Finalized raw exports are renamed atomically by the native worker.
                        data = path.read_bytes()
                        put(tar, relative.as_posix(), data)
                        hashes[relative.as_posix()] = hashlib.sha256(data).hexdigest()
                    replay = Path(__file__).read_bytes()
                    put(tar, "replay-native.py", replay)
                    hashes["replay-native.py"] = hashlib.sha256(replay).hexdigest()
                    put(tar, "SHA256SUMS", "".join(f"{v}  {k}\n" for k, v in sorted(hashes.items())).encode())
            part.close()
            os.replace(temp, output)
        finally:
            temp.unlink(missing_ok=True)


def put(tar, name, data):
    info = tarfile.TarInfo(name)
    info.size = len(data)
    info.mode = 0o644
    tar.addfile(info, io.BytesIO(data))


def unpack(archive, root):
    names = set()
    with tarfile.open(archive, "r|gz") as stream:
        for member in stream:
            name = PurePosixPath(member.name)
            check(member.isfile() and not name.is_absolute() and ".." not in name.parts
                  and str(name) == member.name and name not in names, f"invalid archive member {member.name}")
            names.add(name)
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            with stream.extractfile(member) as original, path.open("xb") as output:
                shutil.copyfileobj(original, output)


def replay(root):
    sums = dict(line.split("  ", 1)[::-1] for line in (root / "SHA256SUMS").read_text().splitlines())
    observed = {p.relative_to(root).as_posix() for p in root.rglob("*") if p.is_file()} - {"SHA256SUMS"}
    check(observed == sums.keys(), "portable file list changed")
    for name, expected in sums.items():
        check(sha(root / name) == expected, f"portable digest differs for {name}")

    sys.path.insert(0, str(root / REL))
    retry = importlib.import_module("import-retry")
    dense, run = retry.dense, retry.run
    def offline_only(*args, **kwargs):
        raise RuntimeError("portable replay must run offline")

    run.ssh = offline_only
    old_path = root / ORIGINAL / "series.json"
    failed = retry.FAILED
    check(sha(old_path) == EXPECTED["original_series"] and sha(failed) == EXPECTED["original_failed_arm"],
          "original native attempt seal differs")
    check(sha(root / REL / "native-request.json") == EXPECTED["original_request"]
          and sha(retry.RETRY) == EXPECTED["retry_request"], "native request seal differs")
    for name, source in ((ID, root / REL / "native-request.json"), (RETRY_ID, retry.RETRY)):
        check((root / QUEUE / f"{name}.json").read_bytes() == source.read_bytes(), "controller intake differs")
    release_path = root / QUEUE / f"{ID}-groups-1-2.released.json"
    release = read(release_path)
    grant, = (root / ORIGINAL).glob("grant-*.json")
    check(sha(release_path) == EXPECTED["original_release"] and release["exit_status"] == 1
          and release["id"] == ID and read(grant)["request_sha256"] == EXPECTED["original_request"],
          "original controller grant or release differs")
    with tarfile.open(failed) as raw:
        error = raw.extractfile("adapter.log").read().decode()
        check("ModuleNotFoundError: No module named 'identity'" in error
              and run.file_json(raw, "status.json")["exit_code"] == 1
              and "adapter/ledger.json" not in raw.getnames(), "first failure receipt differs")
    old = read(old_path)
    packet = read(retry.RETRY)
    launch = read(root / REL / "evidence/import-proof/linux.json")
    check(launch["uid"] == 65534 and launch["gid"] == 65534 and launch["capabilities"] == 0
          and launch["stage_mode"] == "0711" and launch["stopped_before_benchmark"]
          and "ModuleNotFoundError" in launch["old_trace"]
          and "native CPU envelope changed" in launch["repaired_trace"], "Linux permission probe differs")
    staged_sources = dict(read(root / REL / "native-request.json")["local_sha256"], **packet["local_sha256"])
    for name, expected in staged_sources.items():
        if (root / name).is_file():
            check(sha(root / name) == expected, f"changed sealed source: {name}")
    for name, expected in launch["sha256"].items():
        check(sha(root / REL / name) == expected, f"changed Linux probe input: {name}")

    receipt_path = root / D2 / "qualification.json"
    setup = read(receipt_path)
    setup_dir = root / D2 / setup["attempt"]
    qualified_path = setup_dir / "qualified-timing.json"
    check(sha(receipt_path) == old["pins"]["d2_setup"]
          and sha(qualified_path) == setup["inventory_sha256"]
          and sha(setup_dir / "raw.tar.gz") == setup["raw_sha256"]
          and read(setup_dir / "status.json")["exit_code"] == 0, "recovered D2 setup seal differs")
    qualified = read(qualified_path)["lanes"]["r2-d2"]
    d2 = old["config"]["mesh_runner"]["d2-on"]
    assignment, = (arg for arg in qualified["inventory"]["engines"]["mesh"]["argv"]
                   if arg.startswith("COMBINED_RUNNER_ARTIFACTS="))
    check(d2 == {"path": assignment.split("=", 1)[1],
                 **{key: qualified["runner_seal"][key] for key in ("elf", "manifest")}},
          "D2 executable binding differs")

    directory = root / RETRY
    manifest = read(directory / "series.json")
    blocks = list(dense.schedule())
    pins = dict(old["pins"], import_retry_request=EXPECTED["retry_request"],
                original_series=EXPECTED["original_series"], original_failed_arm=EXPECTED["original_failed_arm"],
                **packet["local_sha256"])
    config = old["config"]
    check(manifest["config"] == config and manifest["pins"] == pins
          and manifest["schedule"] == old["schedule"] == blocks, "native recovery changed frozen campaign")
    grants = sorted(directory.glob("grant-*.json"))
    check(grants and all(read(path)["request_sha256"] == EXPECTED["retry_request"] for path in grants),
          "retry controller grant changed")
    dense.stages = retry.stages
    dense.attach_matrix()

    raw_arms = {}
    for path in sorted((directory / "blocks").iterdir()):
        order = read(path / "order.json")
        check(order in blocks, f"unrecognized rotation {path.name}")
        for phase, label, _, folder in dense.matrix.paths(path, config, order):
            raw = folder / "raw.tar.gz"
            if not raw.exists():
                continue
            key = str(folder.relative_to(directory))
            seal = sums[raw.relative_to(root).as_posix()]
            _, inventory = retry.stages(config, label, run.REMOTE / manifest["remotes"][label])
            recorded = read(directory / "staging" / label / "inventory.json")
            check(inventory == recorded, f"staging inventory differs for {label}")
            with tarfile.open(raw) as stream:
                status = run.file_json(stream, "status.json")
            if status["exit_code"] == 0:
                try:
                    score = dense.score_arm(folder, phase, label, config, run.REMOTE / manifest["remotes"][label])
                    check(score["raw_sha256"] == seal, "arm digest changed during replay")
                except dense.matrix.ERRORS as error:
                    raw_arms[key] = dict(sha256=seal, disposition=f"{type(error).__name__}: {error}")
                else:
                    raw_arms[key] = dict(sha256=seal, disposition="reconciled", jobs=score["completed"])
            else:
                raw_arms[key] = dict(sha256=seal, disposition="failed native adapter", exit=status["exit_code"])
    summary = dense.summarize(directory, config, blocks, pins)
    return dict(request=RETRY_ID, first_failure_sha256=EXPECTED["original_failed_arm"],
                first_release_sha256=EXPECTED["original_release"],
                linux_permission_probe=launch["witness"], raw_arms=raw_arms, **summary)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_subparsers(dest="action", required=True)
    modes.add_parser("capture").add_argument("archive", type=Path)
    check_cli = modes.add_parser("check")
    check_cli.add_argument("archive", type=Path)
    check_cli.add_argument("--scratch", type=Path)
    modes.add_parser("replay-tree").add_argument("root", type=Path)
    args = parser.parse_args()
    if args.action == "capture":
        pack(Path(__file__).resolve().parents[4], args.archive.resolve())
        print(sha(args.archive), args.archive)
        return
    if args.action == "check":
        with tempfile.TemporaryDirectory(prefix="dense-native-replay-", dir=args.scratch) as scratch:
            root = Path(scratch).resolve()
            unpack(args.archive, root)
            report = replay(root)
    else:
        report = replay(args.root.resolve())
    print(json.dumps(report, indent=2, sort_keys=True, allow_nan=False))


if __name__ == "__main__":
    main()
