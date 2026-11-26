"""Recompute the net runner CPU available for a change to both direct-pool writes."""
import argparse
import io
import json
import tarfile
from pathlib import Path

from direct4_handler_coverage import control_blocks
from direct4_peer_deadlines import ledger
from warm_river_occupancy import occupancy


def replay(exposure):
    result = {}
    for name, block, read, scores, pins in control_blocks(exposure):
        arms = {}
        for lane in ("service2-a", "service2-b", "direct2", "direct4"):
            path = f"{block}/measured-{lane}/measured-mesh/raw.tar.gz"
            raw = read(path)
            score = scores[lane]
            digest = score["raw_sha256"]
            if pins:
                assert digest == pins[path]
            jobs, _ = ledger(raw, digest)
            assert all(job["hold_ms"] == 0 for job in jobs)
            with tarfile.open(fileobj=io.BytesIO(raw)) as archive:
                def record(filename):
                    return json.load(archive.extractfile(f"adapter/{filename}.json"))
                cpu_ms = 1000 * record("raw")["runner_cpu_seconds_including_startup_and_idle"]
                assert abs(cpu_ms - score["runner_cpu_ms"]) < 0.000001
                statements = record("trusted-application-statements")
                writes = [r for r in statements
                          if r["query"].startswith("INSERT INTO public.mesh_jobs_parity_")]
                assert len(writes) == 2 and all(r["calls"] == r["rows"] == 1024 for r in writes)
                probes = sum(r["calls"] for r in statements if r["query"] == "SELECT $1")
                supply = record("pool-supply")
                assert supply["binding"] == ("direct" if lane.startswith("direct") else "service")
                assert supply["scheduler_workers"] == (4 if lane == "direct4" else 2)
                identities = record("process-identity")
                assert len(identities) == 3 and all(p["executable_sha256"] ==
                    "e836b79a2dbd5ee0e10897de8d3d07fbf789ea038e122d2403799b9879bbbaf4" for p in identities)
            effects = sorted(j["effect_committed_us"] for j in jobs)
            stages = occupancy(jobs, effects[255], effects[767])
            assert stages["middle_512_us"] == score["middle_512_us"]
            arms[lane] = dict(raw_sha256=digest, runner_cpu_ms=cpu_ms, stages=stages,
                              checkout_probes=probes)
        ceiling = max(arms[lane]["runner_cpu_ms"] for lane in ("service2-a", "service2-b"))
        room = ceiling - arms["direct2"]["runner_cpu_ms"]
        result[name] = dict(arms=arms, runner_margin_ms=room, net_us_per_write=1000 * room / 2048)
    assert len(result) == 6
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exposure", type=Path)
    print(json.dumps(replay(parser.parse_args().exposure), indent=2))
