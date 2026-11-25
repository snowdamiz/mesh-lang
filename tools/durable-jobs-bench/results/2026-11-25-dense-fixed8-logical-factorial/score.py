"""Reconcile the assigned native arms and enforce the prospective gate."""

from contextlib import contextmanager
from statistics import median
import tarfile

from contract import STOCK, body, digest

FLOOR = .05
INCREMENT_US = 10_000
METRICS = ("submit_us", "drain_us", "last_effect_from_commit_us",
           "runner_cpu_ms", "pg_cpu_ms", "host_cpu_ms", "terminal_xids")
CONTRACTION = dict(held_contraction_sqlstate="55P03", old_capacity=2, new_capacity=1,
                   executing_after_contraction=2, grants_at_or_above_cap=0, grants_after_release=1)


@contextmanager
def assigned_inventory(transfer, inventory):
    saved = transfer.stages
    transfer.stages = lambda *_: ([], inventory)
    try:
        yield
    finally:
        transfer.stages = saved


def measure(row, pg=None):
    return dict(
        row,
        submit_us=row["last_settlement_us"] - row["first_submit_us"],
        drain_us=row["last_settlement_us"] - row["first_commit_us"],
        last_effect_from_commit_us=row["last_effect_us"] - row["first_commit_us"],
        runner_cpu_ms=row["runner_cpu_ms_per_job"] * 1024,
        host_cpu_ms=(row["pinned_user_cpu_ms_per_job"] + row["pinned_kernel_cpu_ms_per_job"]) * 1024,
        pg_cpu_ms=pg["ms"] if pg is not None else None,
        pg_cpu=pg,
    )


def overlay(meta, lane, expected):
    assert meta["lane"] == lane and meta["observer"] is False
    assert meta["baseline_body_sha256"] == digest(body(STOCK))
    assert meta["body_sha256"] == digest(body(expected))
    acl = meta["metadata"]
    assert acl["owner"] == "parity_migrator" and acl["security_definer"] is False and acl["acl"]
    assert acl["worker_execute"] is True and acl["producer_execute"] is acl["app_execute"] is False
    assert acl["configuration"] == ["search_path=pg_catalog, pg_temp", "lock_timeout=100ms"]


def verify_slots(ledger, row):
    assert ledger["runners"] == row["runners"] == 2
    assert ledger["slots"] == row["slots"] == 64


def score_arm(folder, phase, lane, cfg, grouped, logical, policies):
    transfer, run = grouped.public, grouped.run
    assert run.read_json(folder / "dispatch.json").get("exit_code") == 0
    inventory = cfg["lanes"][lane]["inventory"]
    with assigned_inventory(transfer, inventory):
        if lane in transfer.PEERS:
            result = transfer.score_arm(folder, phase, lane, cfg, None)
            with tarfile.open(folder / "raw.tar.gz") as raw:
                ledger = run.file_json(raw, "adapter/ledger.json")
            terminal_xids = len({transfer.xid(job["settled_xid"], "terminal") for job in ledger["jobs"]})
            return dict(measure(result), terminal_xids=terminal_xids,
                        pg_cpu_scope="peer inventory supplies host and runner CPU")
        writer_lane = "fixed8" if policies[lane] else "stock-a"
        result = grouped.score_arm(folder, phase, writer_lane, dict(cfg, lanes={"hot": cfg["lanes"][lane]}), None)
    with tarfile.open(folder / "raw.tar.gz") as raw:
        assert len(raw.getnames()) == len(set(raw.getnames())), "duplicate archive member"
        def get(name):
            return run.file_json(raw, f"adapter/{name}.json")
        overlay(get("logical-overlay"), lane,
                logical.CANDIDATE.read_text() if lane in {"key", "combined", "smoke-combined"} else STOCK)
        assert not {"adapter/grant-plans.json", "adapter/grant-observer.json"} & set(raw.getnames())
        observed, ledger, row = get("raw"), get("ledger"), get("row")
        verify_slots(ledger, row)
        assert observed["idle_seconds"] >= 3 and observed["raw_idle_sql_calls"] > 0
        assert result["physical_tls_by_role"]["compiled_caller"] == 32
        assert result["checkout_probes"] < 32 and result["peak_connections"] <= 41
        if lane.startswith("smoke-"):
            proof = get("grant-contract")
            assert proof["checks"] and all(proof["checks"].values()) and proof["contraction"] == CONTRACTION
            assert proof["overlay"] == {key: value for key, value in get("logical-overlay").items()
                                        if key not in {"lane", "observer"}}
            return dict(raw_sha256=result["raw_sha256"], completed=result["completed"],
                        native_smoke=proof, writer_mode=policies[lane] or "immediate")
        assert "adapter/grant-contract.json" not in raw.getnames()
        pg = logical.cpu(get("pg-cpu-bracket"), result["first_submit_us"], result["last_settlement_us"])
        assert not pg["unmatched"], "PostgreSQL CPU escaped stable process bracket"
    return measure(result, pg)


def contrast(rows):
    a, b, key, fixed, candidate = (rows[label] for label in ("stock-a", "stock-b", "key", "fixed8", "combined"))
    clocks = {}
    for metric in ("submit_us", "drain_us"):
        faster = min(a[metric], b[metric])
        spread = abs(a[metric] - b[metric])
        saving = faster - candidate[metric]
        increment = min(key[metric], fixed[metric]) - candidate[metric]
        clocks[metric] = dict(saving_us=saving, duplicate_threshold_us=max(FLOOR * faster, spread),
                              incremental_us=increment, incremental_threshold_us=max(INCREMENT_US, spread),
                              passed=saving > max(FLOOR * faster, spread)
                              and increment > max(INCREMENT_US, spread))
    a_xids, b_xids = a["terminal_xids"], b["terminal_xids"]
    xid_saving = min(a_xids, b_xids) - candidate["terminal_xids"]
    checks = dict(
        last_effect_earlier=candidate["last_effect_from_commit_us"] <
            min(a["last_effect_from_commit_us"], b["last_effect_from_commit_us"]),
        fewer_terminal_transactions=xid_saving > max(FLOOR * min(a_xids, b_xids), abs(a_xids - b_xids)),
        no_runner_cpu_transfer=candidate["runner_cpu_ms"] <= max(a["runner_cpu_ms"], b["runner_cpu_ms"]),
        no_host_cpu_transfer=candidate["host_cpu_ms"] <= max(a["host_cpu_ms"], b["host_cpu_ms"]),
        no_pg_cpu_transfer=candidate["pg_cpu_ms"] <= max(a["pg_cpu_ms"], b["pg_cpu_ms"]),
    )
    return dict(clocks=clocks, checks=checks, terminal_xids_saved=xid_saving,
                passed=all(checks.values()) and all(item["passed"] for item in clocks.values()))


def peer_pairs(groups, peers):
    metrics = ("submit_us", "drain_us", "last_effect_from_commit_us", "host_cpu_ms", "runner_cpu_ms", "terminal_xids")
    return {peer: {metric: dict(block_deltas=values, median_delta=median(values))
                   for metric in metrics
                   for values in [[row["measured"][peer][metric] - row["measured"]["combined"][metric]
                                   for row in groups]]}
            for peer in peers}
