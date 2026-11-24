"""Reconcile producer clocks without counting PostgreSQL inclusive times twice."""
from collections import Counter
import json
import math
from statistics import median

from audit import BROKER, times

REQUESTS = ("begin", "wake", "product", "savepoint", "guard", "admission",
            "bulk", "release", "xid", "commit")
PYTHON_EVENTS = ("input_start", "input_read", "input_loaded", "pipe_write_start",
                 "pipe_write_done", "caller_line_ready", "caller_receipt_check_start",
                 "caller_receipt_checked")
RUST_EVENTS = ["broker_input_ready"]
for request in REQUESTS:
    RUST_EVENTS.extend([request + "_send", request + "_sent"])
    if request == "bulk":
        RUST_EVENTS.append("bulk_first_row")
    RUST_EVENTS.append(request + "_ready")
RUST_EVENTS.append("broker_publish")
assert len(RUST_EVENTS) == 33


def points(log):
    marks = {}
    for line in log.splitlines():
        if line.startswith("r2_producer_stage "):
            _, name, ns = line.split()
            assert name not in marks and ns.isdecimal() and int(ns) > 0
            marks[name] = int(ns)
    assert list(marks) == RUST_EVENTS and list(marks.values()) == sorted(marks.values()), marks
    return marks


def segments(log, caller, ledger):
    marks = points(log)
    python = caller["events"]
    assert set(python) == set(PYTHON_EVENTS)
    assert list(map(python.get, PYTHON_EVENTS)) == sorted(python.values())
    marks.update(python)
    # Bound the clock conversion using samples immediately before and after
    # the call. A large step, preemption inside a bracket, or incompatible
    # clock domain refuses the sample instead of publishing a negative stage.
    frames = [caller[name] for name in ("before", "after")]
    for before, wall, after in frames:
        assert 0 <= after - before <= 200_000 and wall > after
    low = min(wall - after for before, wall, after in frames)
    high = max(wall - before for before, wall, after in frames)
    assert high - low <= 250_000
    offset = (low + high) // 2
    for endpoint in ("submit", "commit", "ack"):
        marks[endpoint] = ledger[endpoint + "_us"] * 1000 - offset

    def span(start, end):
        value = marks[end] - marks[start]
        clock_uncertainty = high - low if start in ("submit", "commit", "ack") or end in ("submit", "commit", "ack") else 0
        assert value >= -clock_uncertainty - 1000, (start, end, value)
        return value / 1e6

    # pipe_write_done can race the broker's read completion. Its nested
    # interval is reported separately, outside the disjoint accounting.
    intervals = [("submit", "input_start", "submit_dispatch"),
                 ("input_start", "input_read", "file_read"),
                 ("input_read", "input_loaded", "input_decode"),
                 ("input_loaded", "pipe_write_start", "broker_frame_encode_and_lock"),
                 ("pipe_write_start", "broker_input_ready", "broker_pipe_handoff"),
                 ("broker_input_ready", "begin_send", "broker_input_to_first_database_send")]
    preceding = "begin_send"
    for request in REQUESTS:
        send, sent, ready = (f"{request}_{stage}" for stage in ("send", "sent", "ready"))
        if preceding != send:
            intervals.append((preceding, send, preceding + "_to_" + send))
        if request != "commit":
            intervals.append((send, sent, request + "_send"))
        if request == "bulk":
            intervals.extend([(sent, "bulk_first_row", "bulk_wait_for_first_row"),
                              ("bulk_first_row", ready, "bulk_remaining_result")])
        elif request == "commit":
            intervals.extend([(send, "commit", "commit_send_start_to_record"),
                              ("commit", ready, "commit_record_to_ready")])
        else:
            intervals.append((sent, ready, request + "_wait"))
        preceding = ready
    intervals.extend([("commit_ready", "broker_publish", "ready_to_broker_publish"),
                      ("broker_publish", "caller_line_ready", "broker_publish_to_caller_read"),
                      ("caller_line_ready", "caller_receipt_check_start", "receipt_log"),
                      ("caller_receipt_check_start", "caller_receipt_checked", "caller_receipt_verification"),
                      ("caller_receipt_checked", "ack", "receipt_verification_to_ack")])
    budget = {name: span(start, end) for start, end, name in intervals}
    assert math.isclose(sum(budget.values()), span("submit", "ack"), abs_tol=1e-8)
    return dict(stages_ms=budget, clock_offset_range_ns=[low, high],
                pipe_write_ms=span("pipe_write_start", "pipe_write_done"),
                bulk_wire_ms=span("bulk_send", "bulk_ready"), release_wire_ms=span("release_send", "release_ready"),
                commit_wire_ms=span("commit_send", "commit_ready"))


def sql_witness(stats, mode):
    assert stats["mode"] == mode
    settings = [item for row in stats["role_settings"] for item in row["setconfig"]]
    statements = stats["statements"]
    top, = [row for row in statements if row["toplevel"] and
            row["query"].startswith("SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(")]
    assert top["calls"] == 1 and top["rows"] == 128
    assert top["total_exec_time"] >= 0
    result = dict(bulk_top_execution_ms=top["total_exec_time"])
    for tag, sql in (("release", "RELEASE SAVEPOINT mesh_jobs_enqueue"), ("commit", "COMMIT")):
        stmt, = [row for row in statements if row["toplevel"] and row["query"].strip() == sql]
        assert stmt["calls"] == 1
        result[tag + "_pg_ms"] = stmt["total_exec_time"]
    nested = [row for row in statements if not row["toplevel"]]
    functions = {row["funcname"]: row for row in stats["functions"]}
    if mode == "client":
        assert not settings and not nested and not functions
        return result
    assert set(settings) == {"pg_stat_statements.track=all", "track_functions=pl"}
    layout = functions["enqueue_batch_layout_v1"]
    assert layout["calls"] == 1 and nested
    result["layout_inclusive_ms"] = layout["total_time"]
    # The writable CTE is one nested statement. MERGE, receipt insertion,
    # and any wake triggers it fired are already included in its elapsed time.
    writes, = [row for row in nested if "merge into mesh_jobs.jobs" in row["query"].lower()
               and "insert into mesh_jobs.receipts" in row["query"].lower()]
    assert writes["calls"] == 1 and writes["rows"] == 128
    result["write_cte_inclusive_ms"] = writes["total_exec_time"]
    result["bulk_function_inclusive_ms"] = functions["enqueue_batch_v1"]["total_time"]
    result["wake_trigger"] = functions.get("publish_wake_v1")
    result["functions"] = list(functions.values())
    return result


def observable_contract(read, runner, producer):
    # The old proof deliberately forbids any broker replacement. Carry the
    # same behavior contract here and check the observed launch by its own seal.
    split, identity, row, ledger, policy, database, affinity = map(read, (
        "split-executables.json", "process-identity.json", "row.json", "ledger.json",
        "queue-policy.json", "postgres-settings.json", "process-affinity.json"))
    assert split["checkout_mode"] == "d2" and split["writer_mode"] == "off"
    assert split["producer"] != split["migrator"] and split["producer"] != split["runner"]
    assert (split["broker_sha256"], split["broker_manifest_sha256"]) == (producer["elf"], producer["manifest"])
    assert (split["runner_sha256"], split["runner_manifest_sha256"], split["runner_version"]) == (
        runner["elf"], runner["manifest"], runner["version"])
    assert all(database[k] for k in ("fsync", "full_page_writes", "synchronous_commit", "tls", "track_commit_timestamp"))
    assert database["version"] == "17.11" and database["database_connection_limit"] == 20
    assert identity["cpus"] == [0, 1] and len(affinity) == 2 and all(mask == [0, 1] for mask in affinity.values())
    for label, name, verb, seal in (("broker", "producer", "session", producer), ("runner", "runner", "run", runner)):
        proc = identity["launches"][label]
        assert proc["argv"] == [split[name], verb] and proc["cpus"] == [0, 1]
        assert proc["pid"] > 1 and proc["parent_pid"] == identity["adapter_pid"]
        assert proc["launch_start_ticks"] == proc["start_ticks_before"] == proc["start_ticks_after"] > 0
        assert proc["executable_sha256"] == seal["elf"]
        assert (proc["method"] == "proc-exe" and proc["observed_executable"] == split[name]
                or proc["method"] == "owned-launch" and proc["exe_errno"] in (1, 13))
    assert split["runner_pids"] == [identity["launches"]["runner"]["pid"]]
    assert len({identity["adapter_pid"], identity["launches"]["runner"]["pid"],
                identity["launches"]["broker"]["pid"]}) == 3
    assert row["runners"] == 1 and row["slots"] == 16 and row["connection_cap"] == 20
    assert row["durable_enqueue_transactions"] == 1 and row["completed"] == row["reconciled_jobs"] == 128
    assert row["committed_work_records"] == 256 and row["peak_connections"] == 13
    assert ledger["queue_policy"] == policy["queue_policy"] == row["queue_policy"] == "success_cohort"
    assert policy["admitted_queries"] > 0 and policy["largest_terminal_cohort"] <= 8
    (batch,) = ledger["transactions"]
    xid = str(int(batch["xid"]) % (2**32))
    jobs = ledger["jobs"]
    assert {j["job_key"]: j["id"] for j in jobs} == batch["enqueued_ids"]
    app_xids = {job[key] for job in jobs for key in ("attempt_xid", "effect_xid")}
    terminal = Counter(job["settled_xid"] for job in jobs)
    assert len(app_xids) == 256 and not app_xids.intersection(terminal)
    assert xid not in app_xids and xid not in terminal
    assert len(terminal) == policy["terminal_xids"] and max(terminal.values()) == policy["largest_terminal_cohort"]
    assert all(job["attempts"] == job["effects"] == job["attempt"] == 1
               and job["attempt_committed_us"] < job["effect_committed_us"] <= job["settled_us"] for job in jobs)
    statements = read("trusted-application-statements.json")
    for table in ("attempts", "effects"):
        stmt, = [s for s in statements if s["query"].startswith(f"INSERT INTO public.mesh_jobs_parity_{table}")]
        assert stmt["rows"] == stmt["calls"] == 128
    probes = sum(s["calls"] for s in statements if s["query"] == "SELECT $1")
    assert 0 <= probes < 256
    return dict(checkout_probes=probes)


def read_arm(read, text, mode, runner, producer, check_arm, check_runner_log):
    if mode == "off":
        checked = check_arm(read, "d2", runner, native=True)
    else:
        checked = observable_contract(read, runner, producer)
    broker = producer["elf"]
    check_runner_log(text("runner-0.log"))
    witness = read("producer-writer.json")
    assert witness["bulk"] == 1 and witness["scalar"] == 0 and witness["enabled"]
    split = read("split-executables.json")
    assert split["broker_sha256"] == broker
    assert split["migrator"].endswith("/bulk-enqueue-3b09dc3d0e23-ordinary-v2/mesh")
    sessions = read("postgres-tls-sessions.json")
    tls = Counter(row["role"] for row in sessions)
    assert all(row["tls"] for row in sessions)
    assert tls["parity_producer"] == 1 and tls["compiled_caller"] == 8 and 2 <= tls["parity_subject"] <= 4
    ledger = times(read("ledger.json"))
    result = dict(clock=ledger, enqueue_ms=(ledger["commit_us"] - ledger["submit_us"]) / 1000,
                  ack_ms=(ledger["ack_us"] - ledger["submit_us"]) / 1000,
                  submit_ms=(ledger["last_terminal_us"] - ledger["submit_us"]) / 1000,
                  checkout_probes=checked["checkout_probes"])
    trace = text("broker-errors.log")
    if mode == "off":
        assert "r2_producer_stage" not in trace and broker == BROKER
    else:
        result["wire"] = segments(trace, read("caller-stage.json"), ledger)
        result["postgres"] = sql_witness(read("producer-sql.json"), mode)
    return result


def distribution(values):
    return dict(n=len(values), median=median(values), min=min(values), max=max(values))


def summary(groups):
    assert len(groups) == 5
    verdict = {}
    for field in ("enqueue_ms", "ack_ms", "submit_ms"):
        pairs = []
        for group in groups:
            a, b = (group[key][field] for key in ("off-a", "off-b"))
            mean = (a + b) / 2
            pairs.append(dict(stock_duplicate_spread=abs(a - b),
                              client_minus_stock=group["client"][field] - mean,
                              nested_minus_client=group["nested"][field] - group["client"][field],
                              nested_minus_stock=group["nested"][field] - mean))
        verdict[field] = {name: distribution([p[name] for p in pairs]) for name in pairs[0]}
        verdict[field]["pairs"] = pairs
    return verdict


def attribution(groups, pinned_peers, perturbation):
    """Price only disjoint pre-commit stages that survive the off controls."""
    enqueue = perturbation["enqueue_ms"]
    noise = max(1.0, enqueue["stock_duplicate_spread"]["median"])
    client_pairs = enqueue["pairs"]
    client_usable = (abs(enqueue["client_minus_stock"]["median"]) <= 2.0 and
                     sum(abs(p["client_minus_stock"]) <= max(1.0, p["stock_duplicate_spread"])
                         for p in client_pairs) >= 4)
    nested_usable = client_usable and (
        abs(enqueue["nested_minus_client"]["median"]) <= 2.0 and
        sum(abs(p["nested_minus_client"]) <= max(1.0, p["stock_duplicate_spread"])
            for p in client_pairs) >= 4)
    client = [g["observations"]["client"] for g in groups]
    stages = [row["wire"]["stages_ms"] for row in client]
    # Receipt verification and acknowledgement follow the gap endpoint.
    eligible = {
        "submit_through_first_database_send": (
            "submit_dispatch", "file_read", "input_decode", "broker_frame_encode_and_lock",
            "broker_pipe_handoff", "broker_input_to_first_database_send",
        ),
        "savepoint": ("savepoint_send", "savepoint_wait"),
        "capability_guard": ("guard_send", "guard_wait"),
        "namespace_admission": ("admission_send", "admission_wait"),
        "bulk_wire": ("bulk_send", "bulk_wait_for_first_row", "bulk_remaining_result"),
        "release_savepoint": ("release_send", "release_wait"),
        "xid_lookup": ("xid_send", "xid_wait"),
        "commit_until_record": ("commit_send_start_to_record",),
    }
    priced = {}
    for name, keys in eligible.items():
        values = [sum(row[k] for k in keys) for row in stages]
        priced[name] = dict(distribution(values), values=values,
                            material=client_usable and median(values) >= max(5.0, noise)
                            and sum(value >= max(5.0, pair["stock_duplicate_spread"])
                                    for value, pair in zip(values, client_pairs, strict=True)) >= 4)
    nested = [g["observations"]["nested"]["postgres"] for g in groups]
    nested_components = {key: distribution([row[key] for row in nested]) for key in (
        "bulk_top_execution_ms", "bulk_function_inclusive_ms", "layout_inclusive_ms", "write_cte_inclusive_ms")}
    return dict(client_observer_usable=client_usable, nested_observer_usable=nested_usable,
                estimated_noise_ms=noise, stages=priced, independently_inclusive_sql=nested_components,
                historical_enqueue_gap_us=[p["gap_us"] for p in pinned_peers],
                pending_spec_component_interval=distribution([p["product_ready_to_savepoint_send"] for p in stages]))
