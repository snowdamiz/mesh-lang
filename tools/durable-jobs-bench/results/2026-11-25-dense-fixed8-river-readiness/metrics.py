"""Score dense ledgers and bind startup counters to the actual TLS roster."""

from collections import Counter
import json
from statistics import median

JOBS = 1024
CAP = 96
INPUT = "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"
RIVER = "adapter/river/measured/"
RANKS = (64, 256, 768, 960, JOBS)
EMPTY = dict(jobs=0, attempts=0, effects=0, products=0)


def read(archive, name):
    member = archive.getmember(name)
    assert member.isfile(), name
    with archive.extractfile(member) as stream:
        return json.load(stream)


def samples(archive, engine):
    if engine == "go":
        with archive.extractfile(RIVER + "samples.jsonl") as stream:
            rows = [json.loads(line) for line in stream]
        assert all(row["all_connections_tls"] for row in rows)
        idle, timed = ([row for row in rows if row["idle"] == status] for status in (True, False))
    elif engine == "rust":
        rows = read(archive, "adapter/samples.json")
        idle, timed = rows["idle"], rows["timed"]
        assert all(row["all_tls"] for row in idle + timed)
    else:
        rows = read(archive, "adapter/raw.json")
        idle, timed = rows["idle_samples"], rows["samples"]
    assert idle and timed and all(row["terminal"] == 0 for row in idle) and timed[-1]["terminal"] == JOBS
    assert all(0 < row["connections"] <= CAP for row in idle + timed)
    return dict(idle=[row["connections"] for row in idle], last_idle=idle[-1]["connections"],
                peak=max(row["connections"] for row in timed))


def distribution(values):
    return dict(n=len(values), median=median(values), min=min(values), max=max(values))


def ledger_metrics(ledger, score):
    jobs = ledger["jobs"]
    tx, = ledger["transactions"]
    submit, commit, terminal = (score[name] for name in
                                ("first_submit_us", "first_commit_us", "last_settlement_us"))
    assert len(jobs) == JOBS == score["completed"] and score["failed"] == 0
    assert ledger["input_sha256"] == INPUT and (submit, commit) == (tx["submitted_us"], tx["committed_us"])
    assert terminal == max(job["settled_us"] for job in jobs)
    terminal_xids = {job["settled_xid"] for job in jobs}
    writes = {job[key] for job in jobs for key in ("attempt_xid", "effect_xid")}
    assert len(writes) == 2 * JOBS and not writes & terminal_xids and tx["xid"] not in writes | terminal_xids
    assert all(job["attempts"] == job["effects"] == job["attempt"] == 1 and
               commit <= job["entered_us"] <= job["attempt_committed_us"] <= job["effect_committed_us"]
               <= job["settled_us"] for job in jobs)
    phases = {}
    for name, field in (("first_write", "attempt_committed_us"), ("second_write", "effect_committed_us"),
                        ("settlement", "settled_us")):
        clocks = sorted(job[field] for job in jobs)
        phases[name] = dict(from_commit_us={str(rank): clocks[rank - 1] - commit for rank in RANKS},
                            middle_512_us=clocks[767] - clocks[255],
                            last_64_us=clocks[-1] - clocks[959])
    effect = max(job["effect_committed_us"] for job in jobs)
    return dict(submit_us=terminal - submit, drain_us=terminal - commit,
                enqueue_us=commit - submit, terminal_xids=len(terminal_xids),
                submit_jobs_per_s=JOBS * 1e6 / (terminal - submit), drain_jobs_per_s=JOBS * 1e6 / (terminal - commit),
                after_last_effect_us=terminal - effect, progress=phases,
                per_job_effect_to_settlement_us=distribution([job["settled_us"] - job["effect_committed_us"]
                                                               for job in jobs]))


def cpu(after, before, ticks):
    if before is not None:
        assert (after["pid"], after["start_ticks"]) == (before["pid"], before["start_ticks"])
    deltas = [after[key] - (before[key] if before else 0) for key in ("user_ticks", "system_ticks")]
    assert min(deltas) >= 0 and after["threads"] > 0 and after["rss_bytes"] > 0
    return dict(cpu_ms=sum(deltas) * 1000 / ticks, rss_bytes=after["rss_bytes"], threads=after["threads"])


def startup(archive, mode, score, ticks):
    evidence = read(archive, RIVER + "startup.json")
    producer = read(archive, "adapter/river/producer-startup.json")
    ids = evidence["runner_pids"]
    assert evidence["mode"] == mode and len(set(ids)) == 2
    assert ids == [read(archive, RIVER + f"runner-{i}-capacity.json")["pid"] for i in range(2)]
    points = evidence["points"]
    before, ready, middle, final = points
    assert [row["point"] for row in points] == ["before_runner", "ready", "idle_middle", "before_submit"]
    stamps = [row["epoch_us"] for row in points]
    assert stamps == sorted(set(stamps)) and 0 <= score["first_submit_us"] - stamps[-1] <= 250_000
    assert final["elapsed_us"] - ready["elapsed_us"] >= 2_950_000 and ready["elapsed_us"] > 0
    roster, listeners, postgres = [], [], []
    observer = None
    schema = read(archive, RIVER + "schema.json")
    names = [f"river:{schema}:runner-{index}" for index in range(2)]
    for point in points:
        rows = point["backends"]
        (obs,) = [row for row in rows if row["observer"]]
        assert point["rows"] == EMPTY and 0 < len(rows) <= CAP
        assert all(row["tls"] and not row["superuser"] and row["db_user"] == "parity_subject"
                   and row["database"] == "river" for row in rows)
        assert obs["application_name"] == "river:producer_observer"
        current = (obs["pid"], obs["backend_start"])
        assert observer in (None, current)
        observer = current
        workers = [row for row in rows if not row["observer"]]
        assert all(row["application_name"] in names for row in workers)
        roster.append({row["pid"]: (row["backend_start"], row["application_name"]) for row in rows})
        postgres.append({row["pid"]: row for row in point["postgres_backends"]})
        assert len(roster[-1]) == len(rows) == len(postgres[-1]) and roster[-1].keys() == postgres[-1].keys()
        listeners.append({row["application_name"]: row["pid"] for row in workers if row["last_query_listen"]})
        assert point["producer"]["pid"] == evidence["producer_pid"] == producer["after"]["pid"]
        assert [row["pid"] for row in point.get("runners", [])] == (ids if workers else [])
    assert len(roster[0]) == 1 and listeners[-2] == listeners[-1] and set(listeners[-1]) == set(names)
    fills = []
    for i, name in enumerate(names):
        path = RIVER + f"runner-{i}-pool-fill.json"
        if mode == "cold":
            assert path not in archive.getnames()
            continue
        fill = read(archive, path)
        pids = {pid for pid, (_, app) in roster[-1].items() if app == name}
        assert fill["pool_max"] == 34 and len(fill["backend_pids"]) == len(pids) == 33
        assert set(fill["backend_pids"]) == pids and listeners[-1][name] in pids
        assert stamps[0] <= fill["started_us"] <= fill["finished_us"] <= stamps[1]
        assert 0 <= fill["elapsed_us"] <= 10_000_000
        fills.append(fill["elapsed_us"])
    if mode == "warm":
        assert roster[1] == roster[2] == roster[3] and len(roster[3]) == 67
    else:
        assert len(roster[3]) < 67
    for r, pid in enumerate(ids):
        worker = [point["runners"][r] for point in points[1:]]
        assert all((event["pid"], event["start_ticks"]) == (pid, worker[0]["start_ticks"]) for event in worker)
    costs = [cpu(row, postgres[0].get(pid), ticks) for pid, row in postgres[1].items()]
    return dict(physical_at_submit=len(roster[-1]), listeners=listeners[-1],
                runner_launch_to_ready_us=ready["elapsed_us"], ready_to_presubmit_us=final["elapsed_us"] - ready["elapsed_us"],
                runner_launch_through_settlement_us=score["last_settlement_us"] - ready["epoch_us"] + ready["elapsed_us"],
                per_runner_fill_us=fills, producer_connection_setup_us=producer["elapsed_us"],
                producer_connection=cpu(producer["after"], producer["before"], ticks),
                producer_before_ready=cpu(ready["producer"], before["producer"], ticks),
                runners_at_ready=[cpu(row, None, ticks) for row in ready["runners"]],
                postgres_client_cpu_ms=sum(row["cpu_ms"] for row in costs),
                postgres_client_rss_bytes=sum(row["rss_bytes"] for row in costs))


def comparisons(groups):
    result = {}
    for clock in ("submit_us", "drain_us"):
        rows = []
        for group, measured in sorted(groups.items()):
            mesh, stock, cold, warm = (measured[lane][clock] for lane in
                                       ("combined", "river-2x32", "river-cold", "river-warm"))
            bridge = abs(stock - cold)
            threshold = max(.05 * warm, bridge)
            rows.append(dict(group=group, mesh_us=mesh, stock_us=stock, cold_us=cold, warm_us=warm,
                             stock_minus_mesh_us=stock - mesh, warm_minus_mesh_us=warm - mesh,
                             cold_minus_warm_us=cold - warm, stock_minus_cold_us=stock - cold,
                             mesh_throughput_pct=(warm / mesh - 1) * 100, threshold_us=threshold))
        deltas = [row["warm_minus_mesh_us"] for row in rows]
        verdict = "inconclusive"
        if len(rows) == 6:
            if all(row["warm_minus_mesh_us"] > row["threshold_us"] for row in rows):
                verdict = "Mesh advantage"
            elif all(row["warm_minus_mesh_us"] < -row["threshold_us"] for row in rows):
                verdict = "Mesh deficit"
        fields = ("mesh_us", "stock_us", "cold_us", "warm_us", "stock_minus_mesh_us", "warm_minus_mesh_us",
                  "cold_minus_warm_us", "stock_minus_cold_us", "mesh_throughput_pct")
        result[clock] = dict(blocks=rows, summary={name: distribution([row[name] for row in rows]) for name in fields},
                             mesh_wins=sum(delta > 0 for delta in deltas), verdict=verdict)
    return result
