"""Price one-chunk lookahead using native producer marks and the isolated stock component."""
import hashlib
import json
import tarfile
from pathlib import Path

RESULTS = Path(__file__).resolve().parent.parent
BOUNDARY = RESULTS / "2026-11-24-r2-d2-producer-boundary/evidence"
PREPARATION = RESULTS / "2026-11-24-enqueue-spec-preparation/evidence"
DENSE = RESULTS / "2026-11-25-public-dense-late-capacity/peer-comparison/evidence"
OPPORTUNITIES = 7  # The first of eight ordered batches has no earlier write to overlap.


def checked_marks(series, group, mode):
    arm = f"measured-{mode}/measured-mesh"
    path = series / "blocks" / group["block"] / arm / "raw.tar.gz"
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    assert digest == group["raw_sha256"][arm]
    with tarfile.open(path) as archive:
        marks = {}
        for line in archive.extractfile("adapter/broker-errors.log"):
            if line.startswith(b"r2_producer_stage "):
                _, event, value = line.decode().split()
                assert event not in marks
                marks[event] = int(value)
        writer = json.load(archive.extractfile("adapter/producer-writer.json"))
        assert (writer["bulk"], writer["scalar"]) == (1, 0)
    stages = group["observations"][mode]["wire"]["stages_ms"]
    windows = {}
    for start, end in (("product_ready", "savepoint_send"), ("admission_ready", "bulk_send")):
        name = f"{start}_to_{end}"
        elapsed = marks[end] - marks[start]
        assert elapsed > 0 and abs(elapsed - stages[name] * 1e6) < 0.01
        windows[name] = elapsed
    return dict(group=group["group"], mode=mode, sha256=digest, windows_ns=windows,
                seven_times_both_windows_ns=OPPORTUNITIES * sum(windows.values()))


def dense_sampling():
    release = json.loads((DENSE / "controller-release.json").read_bytes())
    assert release["exit_status"] == 0
    scores = json.loads((DENSE / "scored-before-release.json").read_bytes())["groups"]
    result = []
    for group in range(1, 6):
        path = DENSE / "blocks" / f"{group:02}-noop/measured-B/measured-mesh/raw.tar.gz"
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        assert digest == scores[str(group)]["measured-B"]["raw_sha256"]
        with tarfile.open(path) as archive:
            tx, = json.load(archive.extractfile("adapter/ledger.json"))["transactions"]
            writer = json.load(archive.extractfile("adapter/producer-writer.json"))
            assert (writer["bulk"], writer["scalar"], len(tx["enqueued_ids"])) == (8, 0, 1024)
            stamps = [json.loads(line)["epoch_us"] for line in archive.extractfile("host-samples.jsonl")]
        assert stamps == sorted(set(stamps))
        submit, commit = tx["submitted_us"], tx["committed_us"]
        assert stamps[0] < submit < commit < stamps[-1]
        interior = [stamp - submit for stamp in stamps if submit < stamp < commit]
        result.append(dict(group=group, sha256=digest, enqueue_us=commit-submit,
                           interior_sample_offsets_us=interior))
    return result


def replay():
    series, = (BOUNDARY / "native-timing").iterdir()
    summary = json.loads((series / "summary.json").read_bytes())
    assert summary["complete"] and len(summary["groups"]) == 5
    assert all(not summary["attribution"][mode + "_observer_usable"] for mode in ("client", "nested"))
    linked = json.loads((BOUNDARY / "linked-native-spec-preparation.json").read_bytes())
    component = PREPARATION / "timing-be34d253e19e/raw/summary.json"
    assert hashlib.sha256(component.read_bytes()).hexdigest() == linked["source"]["sha256"]
    assert linked["controller_release"]["exit_status"] == 0 and linked["frame"]["jobs"] == 128
    wall = json.loads(component.read_bytes())["profiles"]["short"]["arm_wall_ms"]
    return dict(short_observed=[checked_marks(series, group, mode) for group in summary["groups"]
                                for mode in ("client", "nested")],
                seven_times_stock_component_wall_ms={
                    mode: {key: OPPORTUNITIES * val for key, val in wall[mode].items()}
                    for mode in ("stock-a", "stock-b")},
                dense_measured=dense_sampling())


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2))
