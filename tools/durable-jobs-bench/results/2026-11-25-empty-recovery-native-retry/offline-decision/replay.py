"""Recover the exact attempted pilot offline through its frozen predicate and scorer."""
import hashlib
import json
from pathlib import Path
import runpy

PACKET = Path(__file__).resolve().parents[1]
OUT = PACKET / "evidence/native"
DIGESTS = {
    "candidate": "a4c6ab38cbac32a7a22ac3cccecf09567063d4d7d4d7942d930b7a2d1dc845a0",
    "gate-stock": "bfb90a17a6a99f0f8b79445482b2ef864c5caa187881c888b599c3975fb4ff38",
    "stock": "46ad3243b3275b62656b698d0ec9cf18cb178ab0b2d4957c18053e8f0a719ee3",
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def replay():
    pilot = runpy.run_path(str(PACKET / "controller.py"))["pilot"]
    release = json.loads((OUT / "controller-release.json").read_text())
    assert release["groups"] == [1] and release["exit_status"] == 1
    assert digest(PACKET / "controller-packet.json") == release["request_sha256"]
    assert digest(OUT / "series.json") == "eea98a0884fa3e8cd39db702cbc36a91f73ed0e7fcb31def575d57f89fd55dc1"
    series = json.loads((OUT / "series.json").read_text())
    cfg = series["config"]
    block, = (OUT / "blocks").iterdir()
    order = json.loads((block / "order.json").read_text())
    assert order == series["plan"][0] == pilot.BLOCKS[0]
    assert set(cfg["lanes"]) == set(order["order"]) == set(DIGESTS)
    arms = {}
    probe = None
    for lane in order["order"]:
        path = block / ("measured-" + lane) / "measured-mesh"
        assert digest(path / "raw.tar.gz") == DIGESTS[lane]
        assert json.loads((path / "dispatch.json").read_text())["exit_code"] == 0
        if lane == "gate-stock":
            probe = pilot.diagnostics(path)
        else:
            try:
                pilot.score(path, "measured", lane, cfg)
            except KeyError as error:
                assert error.args == ("hot",)
            else:
                raise AssertionError("original scorer did not reproduce the archived failure")
            # The historical D2 verifier looks up the policy as "hot" even
            # though the pilot inventories are indexed by treatment.
            bound = dict(cfg, lanes={**cfg["lanes"], "hot": cfg["lanes"][lane]})
            arms[lane] = pilot.score(path, "measured", lane, bound)
    assert probe and set(arms) == {"stock", "candidate"}
    decision = pilot.compare(arms, probe)
    return dict(original_status=release["exit_status"], raw_sha256=DIGESTS, decision=decision,
                plan_gate=probe, arms={k: {field: arm[field] for field in (
                    "completed", "raw_sha256", "submit_ms", "drain_ms", "recovery_submit_to_observer",
                    "stable_worker_postgres_cpu_ms", "stable_worker_postgres_pids")} for k, arm in arms.items()})


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
