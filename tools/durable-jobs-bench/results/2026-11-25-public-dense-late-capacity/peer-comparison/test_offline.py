"""Reconcile legacy stock captures with the exact historical parser and inventories."""
import json
from pathlib import Path
import tempfile

import compare
import pilot

compare.pilot = pilot
ROOT = "path-recovery/peer-stage-recovery"
REVERSE = f"{ROOT}/reverse-continuation/evidence/peers"


def main():
    compare.check()
    score = compare.legacy_score()
    replay = json.loads(pilot.fetch(compare.OLD, f"{pilot.CANONICAL}/{REVERSE}/independent-replay.json"))
    with tempfile.TemporaryDirectory(prefix="peer-parser-", dir=compare.REPO / "target") as temp:
        work = Path(temp)
        for number in (1, 2):
            historical = f"{ROOT}/evidence/peers" if number == 1 else REVERSE
            for label, engine in compare.ENGINES.items():
                capture = work / f"{number}-{label}"
                capture.mkdir()
                archive = f"{pilot.CANONICAL}/{historical}/blocks/{number:02}-noop/measured-{label}/measured-{engine}"
                for name in ("raw.tar.gz", "request.json", "dispatch.json"):
                    (capture / name).write_bytes(pilot.fetch(compare.OLD, f"{archive}/{name}"))
                row = compare.score_peer(capture, engine, "measured", json.loads(compare.stock_bytes(label)), score)
                reference = replay["groups"][str(number)][f"measured-{label}"]
                for key in ("raw_sha256", "submit_us", "drain_us", "runner_cpu_ms", "host_cpu_ms", "topology"):
                    assert row[key] == reference[key], (label, key)
                assert 0 < row["last_effect_us"] <= row["drain_us"]
                assert 0 < row["last_effect_from_submit_us"] <= row["submit_us"]

    measured = {number: {name.removeprefix("measured-"): row for name, row in group.items()
                         if name.startswith("measured-")} for number, group in replay["groups"].items()}
    assert compare.original_peer_gate()(measured) == replay["matched"]
    seals = compare.manifest()
    assert len(seals["schedule"]) == 5
    assert sum(len(order["order"]) * len(order["phases"]) for order in seals["schedule"]) == 70
    print("All six historical peer captures reconcile using their original parser. Five fresh orders check.")


if __name__ == "__main__":
    main()
