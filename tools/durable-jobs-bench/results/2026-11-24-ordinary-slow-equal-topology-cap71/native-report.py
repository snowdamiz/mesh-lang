"""Pure offline replay of committed slow cap-71 native block exports."""

import argparse
from collections import Counter
from hashlib import file_digest
from heapq import heappop, heappush
import json
from pathlib import Path
import re
from statistics import median
import subprocess
import sys
import tarfile

PACKET = Path(__file__).resolve().parent
ENGINES = ("mesh", "go", "oban", "rust")
CAP, JOBS = 71, 512


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read(path):
    return json.loads(path.read_text())


def digest(path):
    with path.open("rb") as source:
        return file_digest(source, "sha256").hexdigest()


def distribution(values):
    return dict(n=len(values), median=median(values), minimum=min(values), maximum=max(values))


def hold_peak(jobs):
    """Count concurrent half-open committed-hold intervals by their starts."""
    endings, peak = [], 0
    for job in sorted(jobs, key=lambda row: row["attempt_committed_us"]):
        start, end = job["attempt_committed_us"], job["effect_committed_us"]
        while endings and endings[0] <= start:
            heappop(endings)
        require(end - start >= 99_000 and job["hold_ms"] == 100, "invalid committed hold")
        heappush(endings, end)
        peak = max(peak, len(endings))
    return peak


def check_seals(block):
    entries = {}
    for line in (block / "SHA256SUMS").read_text().splitlines():
        checksum, name = line.split(maxsplit=1)
        path = (block / name).resolve()
        resolved = path.relative_to(block.resolve()).as_posix()
        require(not (block / name).is_symlink() and resolved not in entries and digest(path) == checksum,
                f"{block.name}: invalid checkpoint seal for {name}")
        entries[resolved] = checksum
    return entries


def inspect_arm(block, arm, score, seals):
    phase, engine = arm.split("-", 1)
    directory = block / arm / arm
    path = directory / "raw.tar.gz"
    for member in (path, directory / "request.json", directory / "dispatch.json"):
        require(member.relative_to(block).as_posix() in seals, f"{arm}: required receipt not sealed")
    require(digest(path) == score["raw_sha256"], f"{arm}: replay and raw bytes differ")
    with tarfile.open(path) as archive:
        def document(name):
            with archive.extractfile(name) as stream:
                return json.load(stream)

        ledger = document("adapter/ledger.json")
        txn, = ledger["transactions"]
        jobs = ledger["jobs"]
        app = [int(j[key]) for j in jobs for key in ("attempt_xid", "effect_xid")]
        terminal = {int(j["settled_xid"]) for j in jobs}
        require(len(set(app)) == 1024 == score["write_xids"] and not set(app) & terminal
                and int(txn["xid"]) not in set(app) | terminal and len(terminal) == score["terminal_xids"],
                f"{arm}: independent write or terminal transactions differ")
        if engine in ("go", "rust"):
            writes = "river/measured/ledger.json" if engine == "go" else "raw-writes.json"
            owners = {item["job_key"]: item["worker_pid"] for item in document("adapter/" + writes)["attempts"]}
        else:
            owners = {j["job_key"]: j["worker_pid"] for j in jobs}
        require(len(jobs) == len(owners) == score["completed"] == JOBS
                and score["failed"] == 0 and ledger["runners"] == 2 and ledger["slots"] == 64
                and ledger["database"]["max_connections"] == ledger["database"]["database_connection_limit"] == CAP,
                f"{arm}: workload or cap differs")
        local = {}
        for pid, count in Counter(owners.values()).items():
            owned = [job for job in jobs if owners[job["job_key"]] == pid]
            local[pid] = dict(jobs=count, hold_peak=hold_peak(owned))
        require(len(local) == 2 and {entry["pid"]: {"jobs":entry["jobs"], "hold_peak":entry["hold_peak"]}
                                     for entry in score["ownership"]} == local
                and all(0 < row["hold_peak"] <= 32 for row in local.values()), f"{arm}: per-runner holds differ")
        last = max(job["settled_us"] for job in jobs)
        origin = txn["committed_us"]
        hold = hold_peak(jobs)
        require(hold == score["committed_hold_peak"] <= 64
                and (last - txn["submitted_us"]) / 1000 == score["submit_ms"]
                and (last - origin) / 1000 == score["drain_ms"], f"{arm}: score or aggregate holds differ")
        sql = score["lifecycle"]
        require(all(sql[key]["calls"] == sql[key]["rows"] == JOBS for key in ("attempt", "effect"))
                and sql["claim"]["rows"] >= JOBS and sql["settlement"]["rows"] == JOBS
                and 0 < len(terminal) == sql["settlement"]["calls"] <= JOBS,
                f"{arm}: SQL write or settlement counters disagree with the ledger")
        connections = score["physical"]
        require(connections["peak"] <= connections["whole_cluster_peak"] <= CAP
                and "parity_owner" not in connections["by_user"], f"{arm}: physical cap or admin window differs")
        for key in ("attempt_committed_us", "effect_committed_us"):
            require(all(origin <= job[key] <= job["settled_us"] for job in jobs), f"{arm}: invalid commit phase")
        return dict(phase=phase, engine=engine, producer_xids=1, application_xids=len(app),
                    terminal_xids=len(terminal), committed_hold_peak=hold, per_pid=local,
                    enqueue_ms=score["first_submit_to_commit_ms"],
                    first_hold_start_ms=(min(job["attempt_committed_us"] for job in jobs) - origin)/1000,
                    last_hold_start_ms=(max(job["attempt_committed_us"] for job in jobs) - origin)/1000,
                    first_hold_end_ms=(min(job["effect_committed_us"] for job in jobs) - origin)/1000,
                    last_hold_end_ms=(max(job["effect_committed_us"] for job in jobs) - origin)/1000,
                    min_hold_ms=min(job["effect_committed_us"] - job["attempt_committed_us"] for job in jobs)/1000,
                    raw_sha256=score["raw_sha256"])


def exclusions(blocks, scored):
    rows = {}
    for block in sorted(blocks.iterdir()):
        if not block.is_dir() or block.name in scored["qualified"]:
            continue
        require(re.fullmatch(r"\d{2}-slow\.\d+", block.name) is not None, f"unexpected attempt {block.name}")
        seals = check_seals(block)
        exports = []
        for path in sorted(block.glob("*/*/raw.tar.gz")):
            require(path.relative_to(block).as_posix() in seals, f"{block.name}: unsealed excluded arm")
            with tarfile.open(path) as archive, archive.extractfile("status.json") as status:
                exports.append(dict(arm=path.parent.name, exit_code=json.load(status)["exit_code"], sha256=digest(path)))
        rows[block.name] = dict(disposition=scored["rejected"].get(block.name, "later attempt than selected"),
                                raw_exports=exports)
    require(set(scored["rejected"]) <= set(rows), "rejected attempts disappeared")
    return rows


def replay_native(directory, *, mode="timing", require_complete=True):
    """Return verified native scores from a moved series directory in a checkout."""
    directory = Path(directory).resolve()
    require(mode in ("setup", "timing"), "unknown replay mode")
    output = subprocess.check_output([sys.executable, "-B", str(PACKET / "sql-audit-retry.py"), "replay",
                                     "--mode", mode, "--directory", str(directory)], text=True)
    scored = json.loads(output)
    require(scored["mode"] == mode and scored["common_cap"] == CAP, "unexpected series identity")
    require(not require_complete or not scored["missing"], f"missing rotations {scored['missing']}")
    series = read(directory / "series.json")
    planned = {block["group"]: block for block in series["plan"]}
    audits = {}
    for name, arms in scored["qualified"].items():
        block = directory / "blocks" / name
        seals = check_seals(block)
        require({"order.json", "reconciled.json"} <= seals.keys()
                and read(block / "reconciled.json") == arms, f"{name}: controller and offline replay differ")
        order = read(block / "order.json")
        require(order == planned[order["group"]] and
                set(arms) == {f"{phase}-{engine}" for phase in order["phases"] for engine in ENGINES},
                f"{name}: planned rotation or roster differs")
        audits[name] = dict(group=order["group"], order=order["order"],
                           arms={key: inspect_arm(block, key, score, seals) for key, score in arms.items()})
    scored.update(arm_audit=audits, excluded_attempts=exclusions(directory / "blocks", scored))
    return scored


def scorecard(replay):
    require(replay["mode"] == "timing", "scorecard requires timing receipts")
    measured = [{engine: (arms[f"measured-{engine}"], replay["arm_audit"][name]["arms"][f"measured-{engine}"])
                 for engine in ENGINES} for name, arms in replay["qualified"].items()]
    paired, phases = {}, {}
    for engine in ENGINES[1:]:
        pairs = [(block["mesh"], block[engine]) for block in measured]
        if not pairs:
            break
        paired[engine] = {}
        for metric in ("submit_ms", "drain_ms"):
            values = [peer[0][metric] - mesh[0][metric] for mesh, peer in pairs]
            known = replay["peer_minus_mesh_ms"][engine][metric]
            require(values == known["by_block"] and median(values) == known["median"],
                    f"{engine}: paired differences do not match raw arm clocks")
            paired[engine][metric] = known
        phase_deltas = {"submit_to_commit": [], "commit_to_last_hold_end": [], "last_hold_end_to_terminal": []}
        for (mesh, mesh_audit), (peer, peer_audit) in pairs:
            submit = peer["first_submit_to_commit_ms"] - mesh["first_submit_to_commit_ms"]
            hold = peer_audit["last_hold_end_ms"] - mesh_audit["last_hold_end_ms"]
            tail = (peer["drain_ms"] - peer_audit["last_hold_end_ms"]) - (mesh["drain_ms"] - mesh_audit["last_hold_end_ms"])
            require(abs(submit + hold + tail - (peer["submit_ms"] - mesh["submit_ms"])) < 1e-7,
                    f"{engine}: paired phase differences do not sum to the submit gap")
            for key, value in zip(phase_deltas, (submit, hold, tail), strict=True):
                phase_deltas[key].append(value)
        phases[engine] = {key: dict(distribution(values), by_block=values) for key, values in phase_deltas.items()}
    metrics = ("submit_jobs_per_s", "commit_jobs_per_s", "submit_ms", "drain_ms", "first_submit_to_commit_ms",
               "runner_cpu_ms_per_job", "pinned_user_cpu_ms_per_job", "pinned_kernel_cpu_ms_per_job",
               "cpu_steal_percent", "cpu_iowait_percent", "peak_executing", "committed_hold_peak",
               "write_xids", "terminal_xids", "entry_to_attempt_commit_p50_ms",
               "attempt_to_effect_commit_p50_ms", "effect_commit_to_settlement_p50_ms")
    rows = {}
    for engine in ENGINES:
        pairs = [block[engine] for block in measured]
        if not pairs:
            break
        arms, audit = zip(*pairs, strict=True)
        rows[engine] = {key: distribution([row[key] for row in arms]) for key in metrics}
        rows[engine].update(
            observed_physical_peak=distribution([row["physical"]["peak"] for row in arms]),
            whole_cluster_peak=distribution([row["physical"]["whole_cluster_peak"] for row in arms]),
            first_hold_start_ms=distribution([row["first_hold_start_ms"] for row in audit]),
            last_hold_end_ms=distribution([row["last_hold_end_ms"] for row in audit]),
            min_hold_ms=distribution([row["min_hold_ms"] for row in audit]),
            sql={kind:{field:distribution([row["lifecycle"][kind][field] for row in arms])
                       for field in ("calls", "rows", "total_exec_time")}
                 for kind in ("attempt", "effect", "claim", "settlement", "lease_or_maintenance")})
    return dict(common_cap=CAP, engines=rows, paired=paired, paired_phase_ms=phases, measured_blocks=len(measured),
                missing=replay["missing"], excluded_attempts=replay["excluded_attempts"])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--mode", choices=("setup", "timing"), default="timing")
    parser.add_argument("--partial", action="store_true", help="inspect a checkpoint before all rotations arrive")
    parser.add_argument("--scores", action="store_true")
    args = parser.parse_args()
    report = replay_native(args.directory, mode=args.mode, require_complete=not args.partial)
    print(json.dumps(scorecard(report) if args.scores else report, sort_keys=True, indent=2, allow_nan=False))
