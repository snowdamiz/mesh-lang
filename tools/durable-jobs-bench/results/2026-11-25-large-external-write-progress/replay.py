"""Partition the large FILE workload clock at durable application writes."""
import hashlib
import json
from pathlib import Path
from statistics import median
import tarfile

HERE = Path(__file__).resolve().parent
NATIVE = HERE.parent / "2026-11-25-public-large-file-external/grant-parser-retry/evidence/native"
LANES = ("file", "river", "oban", "apalis")


def stage_times(jobs, start, committed, settled):
    writes = sorted(j["effect_committed_us"] for j in jobs)
    assert len(writes) == 64
    assert len({j[key] for j in jobs for key in ("attempt_xid", "effect_xid")}) == 128
    assert all(committed <= j["attempt_committed_us"] <= j["effect_committed_us"] <= j["settled_us"]
               for j in jobs)
    assert settled == max(j["settled_us"] for j in jobs)
    return dict(enqueue=committed-start, commit_to_eighth=writes[7]-committed,
                eighth_to_56th=writes[55]-writes[7], after_56th=settled-writes[55])


def replay():
    release = json.loads((NATIVE / "controller-release.json").read_bytes())
    assert release["id"] == "2026-11-25-public-large-file-external-grant-parser-retry"
    assert release["exit_status"] == 0
    blocks = {}
    for path in sorted(NATIVE.glob("blocks/*/reconciled.json")):
        group = int(path.parent.name.split("-")[0])
        assert group not in blocks
        record = json.loads(path.read_bytes())
        rows = {}
        for lane in LANES:
            raw, = path.parent.glob(f"measured-{lane}/*/raw.tar.gz")
            digest = hashlib.sha256(raw.read_bytes()).hexdigest()
            assert digest == record["receipts"]["measured"][lane]["raw_sha256"]
            with tarfile.open(raw) as stream:
                ledger = json.load(stream.extractfile("adapter/ledger.json"))
            clock = record["scores"][lane]
            assert clock["completed"] == 64 and clock["failed"] == 0
            times = stage_times(ledger["jobs"], clock["first_submit_us"], clock["first_commit_us"],
                                clock["last_settlement_us"])
            total = sum(times.values())
            assert total == clock["last_settlement_us"] - clock["first_submit_us"]
            rows[lane] = dict(stages_us=times, submit_us=total, raw_sha256=digest)
        blocks[group] = rows
    assert sorted(blocks) == list(range(1, 6))
    summary = {}
    for peer in LANES[1:]:
        ratios = [row[peer]["submit_us"] / row["file"]["submit_us"] for row in blocks.values()]
        summary[peer] = {
            "file_submit_throughput_ratio_median": median(ratios),
            "wins": sum(r > 1 for r in ratios),
            "at_least_20_percent_faster": sum(r >= 1.2 for r in ratios),
            "file_minus_peer_paired_median_us": {
                stage: median(row["file"]["stages_us"][stage] - row[peer]["stages_us"][stage]
                              for row in blocks.values())
                for stage in blocks[1]["file"]["stages_us"]
            },
        }
    return dict(blocks=blocks, peers=summary)


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
