"""Compare the time to each successive durable effect in the released LARGE FILE pilot."""
from hashlib import sha256
import json
from pathlib import Path
import sys
import tarfile


def replay(pilot):
    score_bytes = (pilot / "scored-before-release.json").read_bytes()
    assert sha256(score_bytes).hexdigest() == "e913615792bb6e7e6cc7da1ec991c99a23d1b11345d2f0e2246ab364e9953e71"
    score = json.loads(score_bytes)
    release = json.loads((pilot / "controller-release.json").read_bytes())
    assert (release["exit_status"], release["request_sha256"], score["advanced"]) == (
        0, "e9de9209cf7f4bac88210a9b8114139de18f4434f16471f323dd5adcfae363a9", False)
    rows = []
    for group in score["groups"]:
        number = group["group"]
        block, = (pilot / "blocks").glob(f"{number:02d}-large.*")
        arms = group["arms"]
        faster_off = min(("off-a", "off-b"), key=lambda k: arms[k]["submit_ms"])
        curves = {}
        for label, expected in group["raw_sha256"]["measured"].items():
            raw, = (block / f"measured-{label}").glob("*/raw.tar.gz")
            assert sha256(raw.read_bytes()).hexdigest() == expected
            with tarfile.open(raw) as archive:
                ledger = json.load(archive.extractfile("adapter/ledger.json"))
            txn, = ledger["transactions"]
            jobs = ledger["jobs"]
            assert len(jobs) == len({j["id"] for j in jobs}) == len({j["effect_xid"] for j in jobs}) == 64
            assert all(j["state"] == "completed" and j["attempts"] == j["effects"] == 1 for j in jobs)
            effects = sorted(j["effect_committed_us"] for j in jobs)
            for clock, start in (("submit", "submitted_us"), ("commit", "committed_us")):
                curve = [t - txn[start] for t in effects]
                marks = arms[label]["landmarks"]
                assert all(curve[n-1] == round(marks[f"effect{n}_from_{clock}_ms"] * 1000) for n in (8, 64))
                curves[label, clock] = curve
        for reference in (faster_off, "producer"):
            for clock in ("submit", "commit"):
                delta = [b - r for b, r in zip(curves["both", clock], curves[reference, clock], strict=True)]
                rows.append(dict(group=number, reference=reference, clock=clock,
                                 later_ranks=[n for n, dt in enumerate(delta, 1) if dt > 0],
                                 tied_ranks=[n for n, dt in enumerate(delta, 1) if dt == 0],
                                 rank_deltas_us=delta))
    return rows


if __name__ == "__main__":
    print(json.dumps(replay(Path(sys.argv[1])), indent=2))
