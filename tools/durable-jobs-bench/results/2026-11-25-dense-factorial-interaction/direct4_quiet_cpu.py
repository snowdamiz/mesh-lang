"""Bound two-core work in the quiet Direct4 arms at the warm River deadlines."""
import argparse
import json
import runpy
import tarfile
from io import BytesIO
from pathlib import Path

from direct4_peer_deadlines import warm_river
from direct4_quiet_cost import RELEASE, REPLAY, read

HOST = runpy.run_path(str(Path(__file__).resolve().parents[1] / "2026-11-25-dense-host-cpu-budget/replay.py"))


def replay(directory):
    release = json.loads(read(directory / "controller-release.json", RELEASE))
    report = json.loads(read(directory / "controller/replay.json", REPLAY))
    request = json.loads(read(directory / "request.json", release["request_sha256"]))
    assert report["complete"] and not report["errors"] and release["exit_status"] == 0
    assert request["clock_ticks"] == 100 and request["cpu"] == [0, 1]
    peers = warm_river()
    result = {}
    for block, phases in report["arms"].items():
        for lane, score in phases["measured"].items():
            path = f"{block}/measured-{lane}/measured-mesh/raw.tar.gz"
            with tarfile.open(fileobj=BytesIO(read(directory / path, report["evidence_sha256s"][path]))) as raw:
                host = json.load(raw.extractfile("host.json"))
                assert host["boot_id"] == request["host"][0] and host["affinity"] == request["cpu"]
                ledger = json.load(raw.extractfile("adapter/ledger.json"))
                samples = [json.loads(line) for line in raw.extractfile("host-samples.jsonl")]
            frames = HOST["frame_bounds"](samples)
            tx, = ledger["transactions"]
            jobs = ledger["jobs"]
            assert len(jobs) == len({job["job_key"] for job in jobs}) == 1024
            settled = max(job["settled_us"] for job in jobs)
            effects = sorted(job["effect_committed_us"] for job in jobs)
            windows = {"submit_us": (tx["submitted_us"], settled),
                       "drain_us": (tx["committed_us"], settled),
                       "middle_512_us": (effects[255], effects[767])}
            metrics = {}
            for metric, (start, end) in windows.items():
                assert end - start == score[metric]
                floor = HOST["work_floor"](frames, start, end, 10_000)
                deadlines = {}
                for group, peer in peers.items():
                    elapsed = peer["phases"]["middle_us"] if metric == "middle_512_us" else peer[metric]
                    deadlines[group] = {"parity_excess_cpu_us": floor - 2 * elapsed,
                                        "twenty_percent_excess_cpu_us": floor - (2 * elapsed * 5 // 6)}
                metrics[metric] = {"elapsed_us": end - start, "host_floor_cpu_us": floor,
                                   "historical_warm_river": deadlines}
            result[path] = metrics
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pilot_directory", type=Path)
    print(json.dumps(replay(parser.parse_args().pilot_directory), sort_keys=True, indent=2))
