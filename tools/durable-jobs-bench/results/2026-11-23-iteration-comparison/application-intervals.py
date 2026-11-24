"""Replay first-write, second-write and settlement timestamps from native ledgers."""

from bisect import bisect_left, bisect_right
from math import isfinite
from pathlib import Path
from statistics import median
import json
import sys
import tarfile

HERE = Path(__file__).resolve().parent
SUITE = HERE.parents[1]
sys.path.insert(0, str(SUITE))
import run
sys.path.insert(0, str(HERE.parent / "2026-11-23-ordinary-native-cpu/replay"))
from offline import host_interval


def stage_times(ledger, score):
    jobs = ledger["jobs"]
    tx, = ledger["transactions"]
    n = len(jobs)
    if n != score["completed"] or n not in (128, 1024):
        raise ValueError("unexpected job count")
    fields = ("entered_us", "attempt_committed_us", "effect_committed_us", "settled_us")
    for job in jobs:
        times = [tx["committed_us"], *(job[key] for key in fields)]
        if times != sorted(times):
            raise ValueError("unordered stage timestamps")
    if len({job[key] for job in jobs for key in ("attempt_xid", "effect_xid")}) != 2 * n:
        raise ValueError("application XIDs are not independent")

    last = [max(job[key] for job in jobs) for key in fields]
    for key, start in (("submit_jobs_per_s", tx["submitted_us"]), ("commit_jobs_per_s", tx["committed_us"])):
        if abs(n * 1_000_000 / score[key] - (last[-1] - start)) > 0.5:
            raise ValueError("recorded end-to-end clock differs")

    effects = sorted(job["effect_committed_us"] for job in jobs)
    trim = n // 10
    lo, hi = effects[trim], effects[n - 1 - trim]
    if hi <= lo:
        raise ValueError("empty middle interval")

    def mean_outstanding(start, end):
        return sum(max(0, min(hi, job[end]) - max(lo, job[start])) for job in jobs) / (hi - lo)

    return dict(enqueue=(tx["committed_us"] - tx["submitted_us"]) / 1000,
                entry=(last[0] - tx["committed_us"]) / 1000,
                write=(last[2] - tx["committed_us"]) / 1000,
                middle_writes_ms=(hi - lo) / 1000,
                sixteenth_write=(effects[15] - tx["committed_us"]) / 1000,
                tail=(last[3] - last[2]) / 1000,
                submit=(last[3] - tx["submitted_us"]) / 1000,
                waiting_first=mean_outstanding(fields[0], fields[1]),
                between_writes=mean_outstanding(fields[1], fields[2]),
                waiting_terminal=mean_outstanding(fields[2], fields[3]))


def application_statement_times(statements, jobs):
    expected = {
        "SELECT $1": ("checkout", 2 * jobs),
        "INSERT INTO public.mesh_jobs_parity_attempts": ("attempt", jobs),
        "INSERT INTO public.mesh_jobs_parity_effects": ("effect", jobs),
    }
    times = {}
    for row in statements:
        kind, count = expected[row["query"].splitlines()[0]]
        if kind in times or row["calls"] != count or row["rows"] != count:
            raise ValueError(f"unexpected application statement counts for {kind}")
        elapsed = row["execution_ms"]
        if not isfinite(elapsed) or elapsed < 0:
            raise ValueError(f"invalid PostgreSQL execution time for {kind}")
        times[kind] = elapsed
    if len(times) != len(expected):
        raise ValueError("incomplete PostgreSQL application statement statistics")
    return times


def drain_disk_flushes(archive, ledger):
    host = run.file_json(archive, "host.json")
    mount = host["pg_mount"]
    if mount["maj:min"] != "8:0" or mount["source"] != "/dev/sda" or mount["fstype"] != "ext4":
        raise ValueError("unexpected native PostgreSQL device")
    with archive.extractfile("host-samples.jsonl") as stream:
        samples = [json.loads(line) for line in stream]
    times = [point["epoch_us"] for point in samples]
    counts, durations = [], []
    for point in samples:
        rows = (s.split() for s in point["diskstats"])
        device, = (row for row in rows if row[:3] == ["8", "0", "sda"])
        counts.append(int(device[18]))
        durations.append(int(device[19]))
    if any(b <= a for a, b in zip(times, times[1:])) or counts != sorted(counts) or durations != sorted(durations):
        raise ValueError("clock or flush counters moved backwards")
    tx, = ledger["transactions"]
    start, end = tx["committed_us"], max(j["settled_us"] for j in ledger["jobs"])
    # worker.py timestamps before reading diskstats. The next sample starts after that read.
    before = bisect_right(times, start) - 2
    after = bisect_left(times, end)
    first_inside = bisect_left(times, start)
    last_inside = bisect_right(times, end) - 2
    if not 0 <= before < first_inside <= last_inside < after < len(times):
        raise ValueError("disk samples do not bracket this drain")
    return [(v[last_inside] - v[first_inside], v[after] - v[before]) for v in (counts, durations)]


def dense_runner_backlog(ledger, topology):
    jobs = ledger["jobs"]
    pids = set(topology["runner_pids_by_index"])
    if (len(jobs) != 1024 or len(pids) != 2 or topology["slots_per_runner"] != 32
            or topology["scheduler_min"] != 2 or topology["scheduler_max"] != 2
            or {j["worker_pid"] for j in jobs} != pids):
        raise ValueError("unexpected two-runner dense roster")
    if any(j["claimed_us"] % 1000 or j["claimed_us"] > j["entered_us"] for j in jobs):
        raise ValueError("unexpected dense claim stamp")
    effects = sorted(j["effect_committed_us"] for j in jobs)
    lo, hi = effects[102], effects[921]
    elapsed = (hi - lo) * len(pids)
    bounds = []
    for rounding in (0, 999):
        queued_time = fewer_than_two = two_or_more = 0
        for pid in pids:
            owned = [j for j in jobs if j["worker_pid"] == pid]
            entered = sorted(j["entered_us"] for j in owned)
            written = sorted(j["effect_committed_us"] for j in owned)
            claimed = sorted(min(j["claimed_us"] + rounding, j["entered_us"]) for j in owned)
            edges = sorted({lo, hi, *(t for t in claimed + entered + written if lo < t < hi)})
            for begin, end in zip(edges, edges[1:]):
                started = bisect_right(entered, begin)
                pending = bisect_right(claimed, begin) - started
                if pending:
                    duration = end - begin
                    queued_time += duration * pending
                    if started - bisect_right(written, begin) >= 2:
                        two_or_more += duration
                    else:
                        fewer_than_two += duration
        bounds.append((queued_time / elapsed, 100 * fewer_than_two / elapsed, 100 * two_or_more / elapsed))
    return [(min(pair), max(pair)) for pair in zip(*bounds)]


def short_connections(archive, engine, completed):
    if engine == "go":
        samples = [json.loads(line) for line in archive.extractfile("adapter/river/measured/samples.jsonl")]
        idle = [row for row in samples if row["idle"]]
        timed = [row for row in samples if not row["idle"]]
    elif engine == "rust":
        samples = run.file_json(archive, "adapter/samples.json")
        idle, timed = samples["idle"], samples["timed"]
    else:
        samples = run.file_json(archive, "adapter/raw.json")
        idle, timed = samples["idle_samples"], samples["samples"]
    if not idle or not timed or any(row["terminal"] != 0 for row in idle) or timed[-1]["terminal"] != completed:
        raise ValueError(f"{engine} connection samples do not bracket the short run")
    if any(not 1 <= row["connections"] <= 20 for row in idle + timed):
        raise ValueError(f"{engine} short connection count outside the physical cap")
    return idle[-1]["connections"], max(row["connections"] for row in timed)


def short_slot_times(ledger):
    slots = ledger["slots"]
    jobs = ledger["jobs"]
    if slots != 16 or len(jobs) != 128 or ledger["runners"] != 1:
        raise ValueError("unexpected short workload dimensions")
    effects = sorted(j["effect_committed_us"] for j in jobs)
    lo, hi = effects[slots - 1], effects[-1]
    entries = sorted(j["entered_us"] for j in jobs)
    if hi <= lo or any(bisect_right(entries, t) - bisect_right(effects, t) > slots for t in entries):
        raise ValueError("invalid handler occupancy or empty short write interval")

    def area(begin, end):
        return sum(max(0, min(hi, j[end]) - max(lo, j[begin])) for j in jobs)

    scale = 1000 * slots
    first = area("entered_us", "attempt_committed_us") / scale
    second = area("attempt_committed_us", "effect_committed_us") / scale
    result = dict(first=first, second=second, other=(hi - lo) / 1000 - first - second)
    if ledger["engine"] != "mesh":
        return result

    if any(j["claimed_us"] % 1000 or j["claimed_us"] > j["entered_us"] for j in jobs):
        raise ValueError("unexpected Mesh claim stamp or entry time")
    ends = sorted(j["settled_us"] for j in jobs)

    def available_before_admission(rounding_us):
        claims = sorted(min(j["entered_us"], j["claimed_us"] + rounding_us) for j in jobs)
        edges = sorted({lo, hi, *(t for t in claims + ends if lo < t < hi)})
        total = 0
        for start, stop in zip(edges, edges[1:]):
            claimed = bisect_right(claims, start)
            settled = bisect_right(ends, start)
            unclaimed = len(jobs) - claimed
            seats = max(0, slots - (claimed - settled))
            total += min(seats, unclaimed) * (stop - start)
        return total / scale

    result["available"] = (available_before_admission(0), available_before_admission(999))
    result["after_write"] = area("effect_committed_us", "settled_us") / scale
    return result


def read_series(series, lanes, *, expected_groups=5):
    inputs = run.read_json(HERE / "series-list.json")
    root = SUITE / next(item["path"] for item in inputs if item["name"] == series)
    if series == "dense-exit-confirmed":
        inventory = run.read_json(root / "series.json")["config"]
        if inventory["cpus"] != [0, 1] or inventory["clock_ticks"] != 100:
            raise ValueError("unexpected Linux CPU or clock tick configuration")
    blocks = run.read_json(HERE / "scores.json")[series]["blocks"]
    if [block["group"] for block in blocks] != list(range(1, expected_groups + 1)):
        raise ValueError(f"incomplete {series}")
    groups = []
    for block in blocks:
        rows = {}
        for lane in lanes:
            score = block["scores"][lane]
            folder = f"measured-{lane}/measured-{score['engine']}"
            raw = root / "blocks" / block["attempt"] / folder / "raw.tar.gz"
            if run.digest(raw) != block["raw_sha256"][folder]:
                raise ValueError(f"raw archive digest differs for {raw}")
            with tarfile.open(raw) as archive:
                ledger = run.file_json(archive, "adapter/ledger.json")
                rows[lane] = stage_times(ledger, score)
                if series == "dense-exit-confirmed":
                    rows[lane]["flushes"], rows[lane]["flush_ms"] = drain_disk_flushes(archive, ledger)
                    effects = sorted(j["effect_committed_us"] for j in ledger["jobs"])
                    with archive.extractfile("host-samples.jsonl") as stream:
                        points = [json.loads(line) for line in stream]
                    rows[lane]["middle_cpu"] = host_interval(points, (effects[102], effects[921]),
                                                             inventory["clock_ticks"], ["cpu0", "cpu1"], 819)
                if series == "dense-equal-topology" and lane == "mesh-v1-2x32":
                    statements = run.file_json(archive, "adapter/trusted-application-statements.json")
                    rows[lane]["statements"] = application_statement_times(statements, score["completed"])
                    rows[lane]["backlog"] = dense_runner_backlog(ledger, run.file_json(archive, "adapter/topology.json"))
                if series == "exit-confirmed-hot-short":
                    rows[lane]["connections"] = short_connections(archive, score["engine"], score["completed"])
                    rows[lane]["slots"] = short_slot_times(ledger)
        groups.append(rows)
    return groups


def report():
    print("# Application writes and durable settlement on the Linode\n")
    print("## Dense jobs with two 32-slot runners per engine\n")
    print("The first three columns count jobs between timestamps. For each job, the replay clips "
          "the interval to the 103rd through 922nd second-write commits, sums the overlapping durations, "
          "and divides by elapsed time. The table shows the median across five rotations. "
          "Each arm committed 2,048 application writes for 1,024 jobs.\n")
    print("| Engine | Entered, awaiting first commit | Between application commits | Second commit to terminal | "
          "Last entry (ms) | Last write (ms) | Final settlement after last write (ms) |")
    print("|---|---:|---:|---:|---:|---:|---:|")
    lanes = {"mesh-v1-2x32": "Mesh V1", "river-2x32": "River", "oban-2x32": "Oban", "apalis-2x32": "Apalis"}
    groups = read_series("dense-equal-topology", lanes)
    for lane, label in lanes.items():
        keys = ("waiting_first", "between_writes", "waiting_terminal", "entry", "write", "tail")
        values = [median(g[lane][key] for g in groups) for key in keys]
        print("| " + " | ".join([label, *(f"{v:,.2f}" for v in values)]) + " |")
    print("\nLast entry and last write are measured from each arm’s durable enqueue commit. The intervals "
          "include scheduling, application code, database work and waits. A peer can continue taking "
          "jobs while earlier terminal acknowledgements are outstanding.\n")
    print("Mesh V1 records entry before decoding the context and calling the application service "
          "for its pool handle. River and Apalis receive a pool with their worker state. "
          "The ledgers have no timestamp for the Mesh service reply. A native comparison that changes only "
          "how the handler receives its pool can price that step. "
          "[Mesh fixture](../2026-11-23-ordinary-trusted-v1/ordinary-trusted-v1.tar.gz) · "
          "[River worker](../../go/worker.go) · [Apalis worker](../../rust/src/main.rs)\n")
    print("## Short jobs with the same bulk Mesh and River binaries\n")
    print("Each row compares 128-job runs. A positive gap means Mesh reached that boundary later. "
          "Last-write gaps use enqueue commit as the origin. The terminal tail measures elapsed time "
          "between the final application commit and the final durable settlement across the arm.\n")
    print("| Series | Rotation | Mesh enqueue (ms) | River enqueue (ms) | Last-write gap (ms) | "
          "Mesh terminal tail (ms) | River terminal tail (ms) | Submit-to-settlement gap (ms) |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|")
    for series, label in (("bulk-hot-short", "Three lanes"), ("bulk-hot-short-peers", "Six lanes")):
        for number, group in enumerate(read_series(series, ("bulk-d2", "river")), 1):
            mesh, river = group["bulk-d2"], group["river"]
            print(f"| {label} | {number} | {mesh['enqueue']:.2f} | {river['enqueue']:.2f} | "
                  f"{mesh['write'] - river['write']:+.2f} | {mesh['tail']:.2f} | {river['tail']:.2f} | "
                  f"{mesh['submit'] - river['submit']:+.2f} |")
    print("\nThe two series ran at different times and in different orders. The observed tails alone "
          "cannot identify the cause of their variation. [Native rates and paired wins](four-engine-scorecard.md) · "
          "[Score inputs and raw archive digests](scores.json)")
    print("\n## PostgreSQL execution time for the dense Mesh handler\n")
    print("Each 2×32 Mesh run recorded 2,048 pool checkout probes, 1,024 attempt inserts and "
          "1,024 effect inserts for its dedicated application role. Every call returned or inserted "
          "one row. The first three times come from PostgreSQL’s cumulative "
          "`pg_stat_statements.total_exec_time` for those calls.\n")
    print("| Rotation | Checkout (ms) | Attempt INSERT (ms) | Effect INSERT (ms) | Total (ms) | "
          "Residual vs River (ms) | Residual vs Oban (ms) | Residual vs Apalis (ms) |")
    print("|---:|---:|---:|---:|---:|---:|---:|---:|")
    for number, group in enumerate(groups, 1):
        mesh = group["mesh-v1-2x32"]
        times = mesh["statements"]
        total = sum(times.values())
        adjusted = mesh["write"] + mesh["tail"] - total
        residuals = [adjusted - (group[p]["write"] + group[p]["tail"])
                     for p in ("river-2x32", "oban-2x32", "apalis-2x32")]
        print(f"| {number} | {times['checkout']:.2f} | {times['attempt']:.2f} | "
              f"{times['effect']:.2f} | {total:.2f} | " +
              " | ".join(f"{gap:+.2f}" for gap in residuals) + " |")
    print("\nThe residual subtracts the entire PostgreSQL execution total from Mesh’s observed "
          "enqueue-commit-to-final-settlement time and compares it with the peer in the same rotation. "
          "Positive means Mesh would still finish later. This credits every millisecond to the "
          "critical path even when backends overlapped and holds all other work fixed. "
          "The counters leave client scheduling, protocol exchanges, planning and transaction "
          "commit time unpriced. They cannot predict the effect of changing concurrency. "
          "[Per-arm archive digests](scores.json) · "
          "[Raw equal-topology series](../2026-11-23-dense-matched-topology/evidence/equal/)")
    print("\n### Dense jobs already claimed by each Mesh runner\n")
    print("The ledger links each job to one of the two runner PIDs. Over the 103rd to 922nd second writes, "
          "the table counts jobs with a database claim stamp that have yet to enter their handler. "
          "The percentages use elapsed time across both runners. Each runner has two scheduler workers.\n")
    print("| Rotation | Mean stamped jobs awaiting entry per runner | Backlog with 0–1 handlers between entry and second write | Backlog with 2+ handlers between entry and second write |")
    print("|---:|---:|---:|---:|")
    for number, group in enumerate(groups, 1):
        pending, fewer, more = group["mesh-v1-2x32"]["backlog"]
        print(f"| {number} | {pending[0]:.2f}–{pending[1]:.2f} | "
              f"{fewer[0]:.1f}–{fewer[1]:.1f}% | {more[0]:.1f}–{more[1]:.1f}% |")
    print("\nThe ranges allow for millisecond claim stamps. A stamp precedes the client’s COMMIT acknowledgement. "
          "The handler interval includes decoding, scheduling and database waits. It measures overlap, "
          "while actual scheduler occupancy and the number of in-flight SQL requests require separate observations.")
    print("\n## Exit-confirmed short jobs against stock peers\n")
    print("The Mesh runner uses the bulk producer, skips checkout probes for recently used "
          "connections and waits for actor teardown before settlement. Each signed difference "
          "is Mesh elapsed time minus the paired peer time. The three intervals partition "
          "the full submit-to-settlement clock. Positive values mean Mesh took longer.\n")
    print("| Rotation | Peer | Submit to enqueue commit (ms) | Enqueue commit to last application write (ms) | "
          "Last write to last settlement (ms) | Full submit difference (ms) |")
    print("|---:|---|---:|---:|---:|---:|")
    groups = read_series("exit-confirmed-hot-short", ("r2-d2", "r2-off", "river", "oban", "apalis"))
    for number, group in enumerate(groups, 1):
        for lane, label in (("river", "River"), ("oban", "Oban"), ("apalis", "Apalis")):
            differences = [group["r2-d2"][k] - group[lane][k] for k in ("enqueue", "write", "tail", "submit")]
            if abs(sum(differences[:3]) - differences[3]) > 0.001:
                raise ValueError("paired intervals do not reproduce submit-to-settlement")
            print(f"| {number} | {label} | " + " | ".join(f"{gap:+.2f}" for gap in differences) + " |")
    print("\nAll four engines committed two independent application transactions per job. "
          "A queue can group terminal acknowledgements and can have an earlier job still waiting "
          "to settle when the last application write commits. [Raw matched short runs]"
          "(../2026-11-24-exit-confirmed-bulk-d2/evidence/native-timing/2026-11-24-exit-confirmed-bulk-d2/)")
    print("\n### Short-job database connections and write progress\n")
    print("The connection columns span all five rotations. Each count covers the queue stack's database "
          "connections, including listeners and the producer. The last two columns split each 128-job "
          "drain at the 16th committed second application write and show the paired runs’ medians.\n")
    print("| Engine | Last idle sample (connections) | Highest measured sample (connections) | "
          "Enqueue commit to 16th second write (ms) | 16th to 128th second write (ms) |")
    print("|---|---:|---:|---:|---:|")
    for key, label in (("r2-d2", "Trusted Mesh · hot checkout"), ("river", "River"),
                       ("oban", "Oban"), ("apalis", "Apalis")):
        rows = [group[key] for group in groups]
        counts = [sorted({row["connections"][i] for row in rows}) for i in (0, 1)]
        spans = [str(v[0]) if v[0] == v[-1] else f"{v[0]}–{v[-1]}" for v in counts]
        first = median(row["sixteenth_write"] for row in rows)
        rest = median(row["write"] - row["sixteenth_write"] for row in rows)
        print(f"| {label} | {spans[0]} | {spans[1]} | {first:.2f} | {rest:.2f} |")
    print("\nThese snapshots show when connections were present. A separate matched run with "
          "River’s connections open before submission can measure how much pool startup changes "
          "its early writes and final settlement.")
    print("\n### Where the 16 slots spent time after the first 16 writes\n")
    print("Each row subtracts a peer’s time from trusted Mesh with hot checkout in the interval "
          "from its own 16th to 128th second write. The columns sum to the elapsed gap before rounding. "
          "They integrate the number of slots in each state over time and divide by 16. "
          "The last state includes admission, settlement and idle slots.\n")
    print("| Rotation | Peer | Handler entry to first write (ms) | Between writes (ms) | "
          "Before handler entry or after second write (ms) | Total difference (ms) |")
    print("|---:|---|---:|---:|---:|---:|")
    for number, group in enumerate(groups, 1):
        for lane, label in (("river", "River"), ("apalis", "Apalis")):
            vals = [group["r2-d2"]["slots"][k] - group[lane]["slots"][k] for k in ("first", "second", "other")]
            print(f"| {number} | {label} | " + " | ".join(f"{v:+.2f}" for v in (*vals, sum(vals))) + " |")
    print("\nThe next table counts empty Mesh slots only while an unclaimed job is available "
          "to fill each one. It excludes empty slots after the last claim. Bounds place each "
          "millisecond claim stamp at the start or end of its bin, capped by handler entry. "
          "A claim stamp is written inside the transaction, before its COMMIT acknowledgement. "
          "The second column for each mode counts slots occupied after the second write through "
          "terminal commit. Both measures use the same 16-slot normalization. They include "
          "required work and cannot establish the speedup from combining settlement and refill.\n")
    print("| Rotation | Hot checkout, free with backlog (ms) | Hot, awaiting settlement (ms) | "
          "Checked checkout, free with backlog (ms) | Checked, awaiting settlement (ms) |")
    print("|---:|---:|---:|---:|---:|")
    for number, group in enumerate(groups, 1):
        cells = []
        for lane in ("r2-d2", "r2-off"):
            row = group[lane]["slots"]
            lower, upper = row["available"]
            cells.extend((f"{lower:.2f}–{upper:.2f}", f"{row['after_write']:.2f}"))
        print(f"| {number} | " + " | ".join(cells) + " |")

    print("\n## Dense jobs after hot checkout\n")
    lanes = {"d2-on": "Mesh · hot checkout", "d2-off-a": "Mesh · checked checkout A",
             "d2-off-b": "Mesh · checked checkout B", "r2-zero": "Mesh · zero-hold writer",
             "r2-off-a": "Mesh · writer off A", "r2-off-b": "Mesh · writer off B",
             "mesh-v1-2x32": "Mesh V1", "oban-2x32": "Oban", "river-2x32": "River",
             "apalis-2x32": "Apalis"}
    groups = read_series("dense-exit-confirmed", lanes, expected_groups=6)
    print("Each value is the median of six native 1,024-job arms. The middle interval begins at the "
          "103rd committed second application write and ends at the 922nd. The last three columns give "
          "time-weighted counts of jobs in each stage during that interval. A job can finish its writes "
          "and free a handler before the queue records its terminal commit.\n")
    print("| Engine | Enqueue (ms) | Middle writes (ms) | Commit to last write (ms) | Terminal tail (ms) "
          "| Entered, awaiting first write | Between writes | Awaiting settlement |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|")
    for lane, label in lanes.items():
        values = [median(g[lane][k] for g in groups) for k in
                  ("enqueue", "middle_writes_ms", "write", "tail", "waiting_first", "between_writes", "waiting_terminal")]
        print(f"| {label} | " + " | ".join(f"{v:.2f}" for v in values) + " |")
    print("\nThe next rows compare hot Mesh with River in each matched rotation. The producer, last-write "
          "and terminal-tail differences sum to the full submit difference. The middle-write column "
          "sits inside the last-write interval. Positive values mean Mesh took longer.\n")
    print("| Rotation | Producer (ms) | Commit to last write (ms) | Terminal tail (ms) | Full submit (ms) "
          "| Middle writes (ms) |")
    print("|---:|---:|---:|---:|---:|---:|")
    for number, group in enumerate(groups, 1):
        mesh, river = group["d2-on"], group["river-2x32"]
        gaps = [mesh[k] - river[k] for k in ("enqueue", "write", "tail", "submit", "middle_writes_ms")]
        if abs(sum(gaps[:3]) - gaps[3]) > 0.001:
            raise ValueError("dense paired intervals do not reproduce submit-to-settlement")
        print(f"| {number} | " + " | ".join(f"{gap:+.2f}" for gap in gaps) + " |")
    print("\n### Device flushes during the dense drain\n")
    print("The recorder reads the PostgreSQL device counters after timestamping each sample. "
          "The next sample bounds when that read finished. Inner differences use two reads fully inside "
          "enqueue commit through final settlement. Outer differences span that whole interval. "
          "Both bounds include every user of the same ext4 device. "
          "[Recorder](../../worker.py) · [Linux 6.8 disk counters](https://www.kernel.org/doc/html/v6.8/admin-guide/iostats.html)\n")
    print("| Rotation | Mesh flushes | River flushes | Mesh flush time (ms) | River flush time (ms) |")
    print("|---:|---:|---:|---:|---:|")
    for number, group in enumerate(groups, 1):
        cells = [group[lane][key] for key in ("flushes", "flush_ms") for lane in ("d2-on", "river-2x32")]
        print(f"| {number} | " + " | ".join(f"{lo}–{hi}" for lo, hi in cells) + " |")
    print("\nLinux counts completed device flushes and accumulates their duration. Several independent "
          "commits can share a flush. Attribution to a specific backend or WAL record requires a PostgreSQL measurement.")
    print("\n### CPU allowance at River’s middle-write pace\n")
    print("The 103rd through 922nd second-write timestamps enclose 819 advances in rank. "
          "Mesh’s busy-CPU lower bound uses readings wholly inside its window with the archived clock and tick allowances. "
          "River’s elapsed time multiplied by two CPUs gives the maximum CPU time available at its observed pace. "
          "Both columns use CPU-ms per advance. The readings cover all activity on the two pinned CPUs. "
          "[CPU bounds replay](../2026-11-23-ordinary-native-cpu/replay/offline.py)\n")
    print("| Rotation | Mesh busy CPU, lower bound | River total CPU allowance | Lower bound minus allowance |")
    print("|---:|---:|---:|---:|")
    excess = []
    for number, group in enumerate(groups, 1):
        lower = group["d2-on"]["middle_cpu"]["host_ms_per_job"]["busy"][0]
        capacity = group["river-2x32"]["middle_cpu"]["capacity_ms_per_job"]
        excess.append(lower - capacity)
        print(f"| {number} | {lower:.3f} | {capacity:.3f} | {excess[-1]:+.3f} |")
    print(f"\nIn {sum(value > 0 for value in excess)} of {len(groups)} rotations, the observed minimum "
          "CPU charge would exceed the time available at River’s middle-write pace even with both CPUs fully busy. "
          "A change could reduce work, move it outside this window, or do both. The samples leave its process and SQL attribution open.")


if __name__ == "__main__":
    report()
