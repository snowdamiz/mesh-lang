"""Verify both controller receipts and replay the recovered native result offline."""
from datetime import datetime
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import sys
import tarfile
from tempfile import TemporaryDirectory

HERE = Path(__file__).resolve().parent
PACKET = HERE.parents[1]
sys.path.insert(0, str(PACKET))
import measure


def read(path):
    return json.loads(path.read_bytes())


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def verify_receipt(folder, request_path, expected_exit):
    request = read(request_path)
    raw = folder / "raw.tar.gz"
    status = read(folder / "status.json")
    release = read(folder / "controller-release.json")
    assert sha(raw) == status["raw_sha256"]
    assert sha(request_path) == status["grant"]["request_sha256"]
    assert request["id"] == release["id"]
    assert release["exit_status"] == status["exit_code"] == expected_exit
    assert release["groups"] == list(range(1, 7)) and release["attempts"] == [expected_exit]
    start, finished, released = map(datetime.fromisoformat,
                                    (release["started"], status["finished"], release["released_at"]))
    assert datetime.fromisoformat(status["grant"]["start"]) <= start <= finished <= released
    pinned = dict(line.split("  ", 1)[::-1] for line in (folder / "INPUTS.sha256").read_text().splitlines())
    assert pinned == request["local_sha256"]
    with tarfile.open(raw) as archive:
        freeze = measure.archive_json(archive, "freeze.json")
        expected_arms = {f"{index:02}-{arm}.json" for index, order in enumerate(measure.ORDERS, 1) for arm in order}
        arms = set(archive.getnames()) & expected_arms
        if expected_exit:
            assert not arms
            log = archive.extractfile("measure.log").read()
            assert b"line 242, in run_arm" in log and b"'int' object has no attribute 'sample'" in log
        else:
            assert arms == expected_arms
    return request, freeze, status


def main():
    path = PACKET / "native-sampler-retry.py"
    spec = importlib.util.spec_from_file_location("native_sampler_retry", path)
    retry = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(retry)
    original = HERE / "native-01"
    recovered = HERE / retry.ATTEMPT
    previous, old_freeze, failed = verify_receipt(original, PACKET / "request.json", 1)
    current, new_freeze, completed = verify_receipt(recovered, retry.PACKET, 0)
    assert failed["raw_sha256"] == retry.FAILED
    assert completed["raw_sha256"] == "05a4dae7e585021a96eddc2b38569ef54b69a758deeb62aba112cbb8585106b5"
    assert sha(PACKET / "request.json") == retry.ORIGINAL
    assert sha(path) == current["retry_driver_sha256"]
    assert sha(retry.MEASURE) == previous["local_sha256"][retry.ARM_PATH]
    for field in ("source_sha", "cpus", "scope", "decision", "prerequisites"):
        assert previous[field] == current[field]
    changed = {name for name in previous["local_sha256"] if previous["local_sha256"][name] != current["local_sha256"][name]}
    assert previous["local_sha256"].keys() == current["local_sha256"].keys() and changed == {retry.ARM_PATH}
    assert {name for name in old_freeze if old_freeze[name] != new_freeze[name]} == {"files"}
    assert {name for name in old_freeze["files"] if old_freeze["files"][name] != new_freeze["files"][name]} == changed
    provenance = read(HERE / "staging-provenance.json")
    for folder, request in ((original, previous), (recovered, current)):
        stage = provenance[folder.name]
        assert stage["source_sha"] == request["source_sha"]
        assert stage["verified_input_members"] == len(request["local_sha256"])
        original_tar = measure.ROOT / stage["original_staged_archive"]
        if original_tar.exists():
            assert sha(original_tar) == stage["input_tar_sha256"]
    measure.TARGET.mkdir(parents=True, exist_ok=True)
    with TemporaryDirectory(dir=measure.TARGET, prefix="archive-replay-") as scratch:
        staged = Path(scratch)
        retry.bundle(staged)
        raw = staged / "raw.tar.gz"
        shutil.copyfile(recovered / raw.name, raw)
        summary = retry.summarize(raw)
    assert json.loads(json.dumps(summary)) == read(recovered / "summary.json")
    wall = summary["paired"]["wall_ms"]
    print(f"{summary['verdict']}: {wall['median_gain_vs_faster_stock']:.3f} ms and "
          f"{wall['gain_beyond_spread']} of 6 rotations beyond duplicate spread; "
          f"{summary['committed_rows']:,} committed rows")


if __name__ == "__main__":
    main()
