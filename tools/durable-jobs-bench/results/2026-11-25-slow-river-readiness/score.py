"""Reconcile the SLOW qualification and replay physical readiness from pglog."""

from collections import Counter
from datetime import datetime, timezone
import json
import tarfile

RIVER = "river/measured/"
ZERO = dict(jobs=0, attempts=0, effects=0, products=0)
OBSERVER = "river:producer_observer"
FAULT = "frozen trial: physical readiness requires 33 clients per runner at ready, observed 33/32"


def events(base, log):
    for line in log.splitlines():
        match = base.equal.CONNECT.match(line)
        if not match:
            continue
        stamp = datetime.fromisoformat(match["at"]).replace(tzinfo=timezone.utc)
        when = int(stamp.timestamp()) * 1_000_000 + stamp.microsecond
        name = None
        if match["event"] == "connection authorized:":
            auth = base.equal.AUTH.search(match["detail"])
            base.require(auth is not None, "missing PostgreSQL authorization identity")
            name = (*auth.groups(), "SSL enabled" in match["detail"])
        yield when, int(match["pid"]), name


def roster(base, log_events, start, end):
    live = {}
    for when, pid, identity in log_events:
        if when > end // 1000 * 1000:
            break
        if when >= start // 1000 * 1000:
            # The log has millisecond precision. A change in an endpoint
            # bucket is ambiguous, so the proof accepts only a stable roster.
            raise ValueError("physical PostgreSQL roster changed near or within the readiness window")
        if identity is None:
            del live[pid]
        else:
            base.require(pid not in live, "duplicate PostgreSQL authorization")
            live[pid] = identity
    result = {}
    for pid, (user, database, app, tls) in live.items():
        base.require(user == "parity_subject" and database == "river" and tls and app,
                     "readiness interval includes a foreign or non-TLS physical session")
        result[pid] = app
    return result


def startup(base, files, text, mode, row, ledger=None):
    fault = mode == "underready"
    doc = files(RIVER + ("startup-partial.json" if fault else "startup.json"))
    schema = files(RIVER + "schema.json")
    runner_names = [f"river:{schema}:runner-{i}" for i in (0, 1)]
    names = ["before_runner", "ready"] if fault else ["before_runner", "ready", "idle_middle", "before_submit"]
    points = doc["points"]
    ids = doc["runner_pids"]
    base.require(doc["mode"] == mode and [p["point"] for p in points] == names
                 and len(ids) == len(set(ids)) == 2 and doc["producer_pid"] not in ids,
                 "rebuilt startup probe sequence or process roster differs")
    stamps = [p["epoch_us"] for p in points]
    base.require(stamps == sorted(set(stamps)), "startup evidence clock is not increasing")
    if ledger is not None:
        base.require(stamps[-1] <= ledger["transactions"][0]["submitted_us"]
                     and ids == [files(RIVER + f"runner-{i}-capacity.json")["pid"] for i in (0, 1)],
                     "startup probe or runner identities differ from the scored workload")
    log_events = list(events(base, text("physical-postgres.log")))
    proven = []
    listener_ids = []
    for point in points:
        stamp = point["epoch_us"]
        identities = {b["pid"]: b for b in point["backends"]}
        live = roster(base, log_events, stamp, stamp)
        expected = {pid: b["application_name"] for pid, b in identities.items()}
        base.require(len(identities) == len(point["backends"]) and live == expected
                     and point["rows"] == ZERO and all(b["tls"] and not b["superuser"]
                         and b["db_user"] == "parity_subject" and b["database"] == "river"
                         and b["backend_start"] for b in identities.values()),
                     f"physical roster or empty workload differs at {point['point']}")
        observers = [pid for pid, b in identities.items() if b["observer"]]
        base.require(len(observers) == 1 and live[observers[0]] == OBSERVER
                     and set(live.values()) <= {OBSERVER, *runner_names}
                     and list(live.values()).count(OBSERVER) == 1, "observer identity differs")
        listeners = {pid: live[pid] for pid, b in identities.items() if b["last_query_listen"]}
        if point["point"] == "before_runner":
            base.require(live == {observers[0]: OBSERVER} and not listeners, "runner opened prematurely")
        else:
            base.require(list(sorted(listeners.values())) == runner_names, "unmatched detached River listeners")
            listener_ids.append(listeners)
        proven.append(identities)
    base.require(all(l == listener_ids[0] for l in listener_ids), "detached listener identities changed")
    ready = points[1]
    base.require(all(p["producer"]["pid"] == doc["producer_pid"] for p in points)
                 and all([r["pid"] for r in p["runners"]] == ids for p in points[1:]),
                 "startup process identities differ")
    ready_ids = {pid: b["application_name"] for pid, b in proven[1].items()}
    if mode in ("warm", "underready"):
        expected_sizes = (33, 32 if fault else 33)
        fill_stamps, release_stamps, fill_reports = [], [], []
        for i, name in enumerate(runner_names):
            fill = files(RIVER + f"runner-{i}-pool-fill.json")
            close = files(RIVER + f"runner-{i}-pool-barrier.json")["released_after_us"]
            seen = {b["pid"]: b["backend_start"] for b in fill["backends"]}
            actual = {pid: b["backend_start"] for pid, b in proven[1].items() if b["application_name"] == name}
            base.require(len(seen) == len(fill["backends"]) == expected_sizes[i] and seen == actual
                         and fill["pool_max"] == 34 and stamps[0] <= fill["started_us"] <= fill["held_us"] <= close
                         <= stamps[1], "real held TLS sessions do not match runner startup or configured pool")
            fill_stamps.append(fill["held_us"])
            release_stamps.append(close)
            fill_reports.append(dict(runner=i, held=len(seen), elapsed_us=fill["held_us"] - fill["started_us"]))
        barrier = max(fill_stamps)
        finish = stamps[-1] if fault else ledger["transactions"][0]["submitted_us"]
        base.require(barrier <= min(release_stamps) and roster(base, log_events, barrier, finish) == ready_ids
                     and all(p == proven[1] for p in proven[2:]), "physical warm roster did not survive to readiness")
        row["pool_fills"] = fill_reports
    else:
        base.require(not any(f"runner-{i}-pool-fill.json" in text.names for i in (0, 1))
                     and len(proven[-1]) == 7 and len(proven[1]) <= 7
                     and roster(base, log_events, stamps[-1], ledger["transactions"][0]["submitted_us"])
                     == {pid: b["application_name"] for pid, b in proven[-1].items()},
                     "rebuilt lazy River differs from stock's pre-submit policy")
    row.update(ready_physical_clients=len(proven[1]), last_probed_physical_clients=len(proven[-1]),
               ready_detached_listeners=listener_ids[0], startup_probe_points=points,
               producer_connection_setup=files("river/producer-startup.json"), runner_launch_to_ready_us=ready["elapsed_us"])
    if fault:
        row["physical"] = base.equal.physical(text("physical-postgres.log"), "go", stamps[1], stamps[1])
    if ledger is not None:
        origin = ready["epoch_us"] - ready["elapsed_us"]
        base.require(stamps[0] <= origin <= ready["epoch_us"], "runner launch origin not bracketed")
        row["runner_launch_to_last_settlement_us"] = row["last_settlement_us"] - origin


def landmarks(base, files, row, ledger):
    txn, = ledger["transactions"]
    writes = sorted(job["attempt_committed_us"] for job in ledger["jobs"])
    effect = max(job["effect_committed_us"] for job in ledger["jobs"])
    terminal = row["last_settlement_us"]
    commit = txn["committed_us"]
    base.require(len(writes) == 512 and commit <= writes[63] <= writes[447] <= writes[-1] <= effect <= terminal,
                 "durable SLOW progress order differs")
    row["progress_us"] = {
        name: dict(submit=stamp - txn["submitted_us"], drain=stamp - commit)
        for name, stamp in (("first_write_64", writes[63]), ("first_write_448", writes[447]),
                            ("first_write_512", writes[-1]), ("last_effect", effect), ("terminal", terminal))
    }
    row["middle_first_writes_us"] = writes[447] - writes[63]
    row["terminal_tail_us"] = terminal - effect
    marks = files("startup-clock.json")
    base.phases_and_readiness(None, ledger, "diagnostic", "mesh", marks, row, True)
    if "runner_launch_to_last_settlement_us" not in row:
        # This is the wrapper's dispatch observation. River stock dispatch
        # includes Go adapter migration. Mesh and Oban observe first fork.
        row["runner_dispatch_to_last_settlement_us"] = terminal - marks["runner_launch_dispatch"]["epoch_ns"] // 1000


def score(base, folder, label, cfg, phase):
    lane = cfg["lanes"][label]
    engine, inventory = lane["engine"], lane["inventory"]
    spec = inventory["engines"][engine]
    mode = spec["argv"][3]
    raw_folder = folder / f"{phase}-{engine}"
    raw = raw_folder / "raw.tar.gz"
    row = {}
    if mode != "underready":
        row = base.equal.score(raw_folder, engine, phase, inventory, cfg["cap71_design"])
    with tarfile.open(raw, "r:gz") as archive:
        def files(name):
            return base.matrix.file_json(archive, "adapter/" + name)

        def read_text(name):
            with archive.extractfile("adapter/" + name) as stream:
                return stream.read().decode()

        read_text.names = {name.removeprefix("adapter/" + RIVER) for name in archive.getnames()
                           if name.startswith("adapter/" + RIVER)}
        if mode == "underready":
            native = base.matrix.file_json(archive, "invocation.json")
            host = base.matrix.file_json(archive, "host.json")
            status = base.matrix.file_json(archive, "status.json")
            error = files("river/status.json")
            config = files("river/config.json")
            base.require(native["attempt"] == base.read(raw_folder / "request.json")["attempt"]
                         and (native["engine"], native["profile"], native["phase"]) == ("go", "slow", phase)
                         and host["verified_files"] == spec["verified_files"] and host["affinity"] == [0, 1]
                         and [host["boot_id"], host["kernel"], host["pg_mount"]["maj:min"]] == cfg["cap71_design"]["host"]
                         and status["exit_code"] == 1 and error["ok"] is False and error["error"] == FAULT
                         and config["binary_sha256"] == spec["artifact_sha256"]
                         and config["profile_config"] == dict(runners=2, slots=64, physical_connection_cap=71,
                             full_unmeasured_warmup=True, input_sha256=cfg["cap71_design"]["input_sha256"])
                         and not read_text(RIVER + "enqueue.jsonl")
                         and "adapter/ledger.json" not in archive.getnames(),
                         "under-ready arm did not stop on the deliberate physical shortage before submission")
            startup(base, files, read_text, mode, row)
            row["physical_readiness_rejection"] = error["error"]
        else:
            ledger = files("ledger.json")
            pg = files("codec-postgres.json")
            settings = {key: "on" for key in ("ssl", "fsync", "synchronous_commit", "full_page_writes",
                                               "track_commit_timestamp")}
            settings.update(server_version_num="170011", default_toast_compression="pglz", max_connections="71",
                            reserved_connections="0", superuser_reserved_connections="0",
                            shared_preload_libraries="pg_stat_statements")
            base.require(pg == dict(path=cfg["postgres"]["prefix"] + "/bin/postgres", sha256=base.PG_SHA,
                                    settings=settings), "cap-71 PostgreSQL identity or codec differs")
            receipt = files("payload-storage.json")
            security = receipt["security"]
            base.require(security["observer_tls"] and security["database_limit"] == 71 and security["codec_overrides"] == 0,
                         "TLS, codec override or database cap differs")
            row.update(base.codec_packet.original.verify_storage(receipt, ledger, engine, "pglz"))
            row["compression_counts"] = base.verify_codec(base.codec_packet.storage, engine, receipt["persisted"],
                                                          ledger["jobs"], "pglz")
            if mode in ("cold", "warm"):
                startup(base, files, read_text, mode, row, ledger)
            elif mode == "stock":
                starts = files("startup-clock.json")
                base.phases_and_readiness(archive, ledger, phase, engine, starts, row, True)
                log_events = list(events(base, read_text("physical-postgres.log")))
                submit = ledger["transactions"][0]["submitted_us"]
                ready = starts["runners_ready"]["epoch_ns"] // 1000
                live = roster(base, log_events, ready, submit)
                schema = files(RIVER + "schema.json")
                base.require(Counter(live.values()) == {OBSERVER: 1, **{f"river:{schema}:runner-{i}": 3 for i in (0, 1)}},
                             "retained stock idle roster differs")
                row["ready_physical_clients"] = len(live)
            landmarks(base, files, row, ledger)
            with archive.extractfile("host-samples.jsonl") as stream:
                points = [json.loads(line) for line in stream]
            span = points[-1]["monotonic_ns"] - points[0]["monotonic_ns"]
            base.require(span > 0, "nonpositive deployment envelope")
            row["recorder_wall_ms"] = span / 1_000_000
    row["attempt_id"] = base.read(raw_folder / "request.json")["attempt"]
    row["raw_sha256"] = base.sha(raw)
    return row
