"""Replay the two-runner peer functional smokes and native launch rosters."""

import argparse
from collections import Counter
import json
from pathlib import Path
import sys
import tarfile

from matrix import PACKET, SUITE, WORK, diagnostic_reconciler, run, require


def launch_receipt(engine, roster, ledger):
    entries = list(roster.values())
    pids = {int(pid) for pid in roster} if engine == "oban" else {row["pid"] for row in entries}
    require(len(roster) == len(pids) == 2 and all(row["slots"] == 32 for row in entries),
            f"{engine} did not launch two 32-slot runners")
    if engine == "oban":
        require({job["worker_pid"] for job in ledger["jobs"]} <= pids
                and {row["index"] for row in entries} == {0, 1}, "Oban runner rosters and app PIDs differ")
    return roster


def local():
    sys.path.insert(0, str(SUITE / "go"))
    import raw_evidence
    from verify import PROFILES

    go = WORK / "local-river/river"
    raw_evidence.PROFILES = {**PROFILES, "noop": (2, *PROFILES["noop"][1:])}
    config = run.read_json(go / "config.json")
    ledger, _ = raw_evidence.verify_run(go, "noop", config["binary_sha256"], smoke=False)
    roster = [run.read_json(go / "measured" / f"runner-{i}-capacity.json") for i in range(2)]
    require(len(ledger["jobs"]) == 1024 and ledger["runners"] == len(roster) == 2
            and all(row["slots"] == 32 for row in roster), "wrong local River topology or completion count")
    results = {"go": dict(jobs=1024, topology=roster, native_rss=ledger["peak_runner_rss_bytes"],
                          adapter_status=run.read_json(go / "status.json"),
                          platform_limit="Stock adapter requires Linux RSS to emit the final scoring ledger.")}
    for engine, name in (("oban", "local-oban-02"), ("rust", "local-apalis")):
        folder = WORK / name
        ledger = run.read_json(folder / "ledger.json")
        artifact = {field: ledger[field] for field in ("version", "artifact_sha256")}
        score = diagnostic_reconciler(2)(ledger, engine, "noop", artifact)
        require(score["completed"] == 1024, f"{engine} did not complete 1024 jobs")
        results[engine] = dict(jobs=score["completed"],
            topology=launch_receipt(engine, run.read_json(folder / "runner-topology.json"), ledger))
    return results


def native(series):
    config = run.read_json(series / "series.json")["config"]
    results = {}
    for path in sorted((series / "blocks").glob("*")):
        if not (path / "reconciled.json").is_file():
            continue
        for phase in ("warmup", "measured"):
            for engine, lane in (("oban", "oban-2x32"), ("rust", "apalis-2x32")):
                folder = path / f"{phase}-{lane}" / f"{phase}-{engine}"
                with tarfile.open(folder / "raw.tar.gz", "r:gz") as archive:
                    roster = run.file_json(archive, "adapter/runner-topology.json")
                    ledger = run.file_json(archive, "adapter/ledger.json")
                    if engine == "oban":
                        reports = run.file_json(archive, "adapter/oban-emulator.json")
                        require(set(map(int, roster)) == {row["pid"] for row in reports},
                                "Oban launch and live BEAM PIDs differ")
                    results[str(folder.relative_to(series))] = launch_receipt(engine, roster, ledger)
    return results


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("series", type=Path, nargs="?")
    args = parser.parse_args()
    print(json.dumps(native(args.series) if args.series else local(), indent=2, sort_keys=True))
