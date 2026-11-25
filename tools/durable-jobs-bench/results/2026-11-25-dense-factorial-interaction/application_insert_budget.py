"""Credit both full-workload INSERT executors against each matched middle-write budget."""
import json
import math
import runpy

from settlement_scans import FIXED8, RESULTS, archive, drain, read
from tls_role_ticks import summarize

HOST = runpy.run_path(str(RESULTS / "2026-11-25-dense-host-cpu-budget/replay.py"))


def middle(raw, score):
    _, _, jobs = drain(raw)
    times = sorted(job["effect_committed_us"] for job in jobs)
    left, right = times[255], times[767]
    assert right - left == score["progress"]["second_write"]["middle_512_us"]
    return left, right


def replay():
    groups = json.loads((FIXED8 / "replay-rest.json").read_text())["groups"]
    results = []
    for group in range(1, 7):
        scores = groups[str(group)]["measured"]
        mesh, river = scores["combined"], scores["river-warm"]
        with archive(FIXED8, group, "river-warm", river["raw_sha256"]) as raw:
            left, right = middle(raw, river)
            peer_capacity = 2 * (right - left)
        with archive(FIXED8, group, "combined", mesh["raw_sha256"]) as raw:
            left, right = middle(raw, mesh)
            points = [json.loads(line) for line in raw.extractfile("host-samples.jsonl")]
            floor = HOST["work_floor"](HOST["frame_bounds"](points), left, right, 10_000)
            roster = summarize(raw)
            assert roster["roster_complete"]
            ticks = roster["roles"]["compiled_caller"]["per_backend_ticks"]
            statements = read(raw, "adapter/trusted-application-statements.json")
            inserts = {}
            for table in ("attempts", "effects"):
                prefix = "INSERT INTO public.mesh_jobs_parity_" + table
                statement, = (s for s in statements if s["query"].startswith(prefix))
                assert statement["calls"] == statement["rows"] == 1024
                inserts[table] = statement["execution_ms"]
            # Credit the entire arm even though only part of either statement
            # total can belong to the middle window.
            credit = math.ceil(1000 * sum(inserts.values()))
            results.append(dict(group=group, mesh_raw_sha256=mesh["raw_sha256"],
                                river_raw_sha256=river["raw_sha256"], insert_executor_ms=inserts,
                                app_backend_ticks=sum(ticks), backends_with_ticks=sum(t > 0 for t in ticks),
                                mesh_middle_us=right-left, warm_river_middle_us=peer_capacity//2,
                                host_work_floor_us=floor, credit_us=credit,
                                excess_over_two_cpu_peer_capacity_us=floor-peer_capacity,
                                excess_after_insert_credit_us=floor-peer_capacity-credit))
    return results


if __name__ == "__main__":
    print(json.dumps(replay(), sort_keys=True, indent=2))
