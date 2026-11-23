#!/usr/bin/env python3
"""Run the bounded claim-cursor screen in a private PG17.11 Unix-socket cluster."""
import argparse
import csv
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import select
import subprocess
import tempfile

ROOT = Path.cwd()
BENCH = ROOT / "tools/durable-jobs-bench/results/2026-11-23-native-worker-cost"
SCHEMA = ROOT / "compiler/mesh-rt/src/jobs/schema.sql"
STORE = SCHEMA.with_name("store.rs")
FIXTURE = Path(__file__).resolve().with_name("fixture.sql")
TRACE = """
SET auto_explain.log_min_duration = 0;
SET auto_explain.log_nested_statements = on;
SET auto_explain.log_analyze = on;
SET auto_explain.log_timing = off;
SET auto_explain.log_buffers = on;
SET auto_explain.log_verbose = on;
SET auto_explain.log_parameter_max_length = 0;
SET auto_explain.log_format = 'json';
"""
MODES = ("force_custom_plan", "force_generic_plan", "auto")
# name, frozen payload profile, completed, running, requested slots, locked ready positions, mixed eligibility
CASES = (
    ("dense_early", "noop", 0, 0, 8, (), False),
    ("dense_middle", "noop", 512, 56, 8, (), False),
    ("dense_late", "noop", 992, 31, 8, (), False),
    ("large_early", "large", 0, 0, 8, (), False),
    ("large_middle", "large", 32, 16, 4, (), False),
    ("large_late", "large", 59, 4, 1, (), False),
    ("sparse", "sparse", 10, 0, 1, (), False),
    ("mixed", "sparse", 10, 0, 3, (11, 12), True),
    ("dense_locked", "noop", 0, 0, 8, tuple(range(1, 25)), False),
)
INPUT_DIGESTS = {
    "noop": "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a",
    "large": "9d7f130f2cc690487582f73c9bea5fbb1f2661f8e416beb2f1574a6f868cc5c1",
    "sparse": "20225faf6274696987b2192136f344d69be243f0a21dc9afad71e714c6ee08c5",
}
VARIANTS = {"stock": "", "limit20": "LIMIT 20 ", "limit_remaining": "LIMIT (remaining + 4) "}
SEP = "\x1f"
PATH_FIELDS = {
    "Node Type", "Join Type", "Index Name", "Relation Name", "Plan Rows", "Plan Width",
    "Startup Cost", "Total Cost", "Actual Rows", "Actual Loops", "Rows Removed by Filter",
    "Rows Removed by Index Recheck", "Rows Removed by Join Filter", "Index Cond", "Filter",
    "Join Filter", "Hash Cond", "Sort Method", "Sort Space Used", "Sort Space Type",
    "Shared Hit Blocks", "Shared Read Blocks", "Shared Dirtied Blocks", "Shared Written Blocks",
    "Temp Read Blocks", "Temp Written Blocks", "Hash Batches",
}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def sql_string(value):
    return "'" + value.replace("'", "''") + "'"


def checked(argv, *, env, source=None):
    result = subprocess.run(argv, env=env, input=source, capture_output=True, timeout=60)
    if result.returncode:
        raise RuntimeError(f"{argv[0]} exited {result.returncode}: {result.stderr.decode()[-4000:]}")
    return result.stdout


class Cluster:
    def __init__(self, work, sock, bindir):
        self.work = work
        self.bindir = bindir
        self.data = work / "pgdata"
        self.sock = sock
        assert len(os.fsencode(self.sock / ".s.PGSQL.5432")) < 104, "use a shorter output path"
        self.env = {key: value for key, value in os.environ.items() if not key.startswith("PG")}
        self.env.update(TMPDIR=str(work), PGHOST=str(self.sock), PGPORT="5432", PGUSER="probe_owner",
                        PGAPPNAME="setup", LC_ALL="C")

    def command(self, name, *args, source=None):
        return checked([str(self.bindir / name), *map(str, args)], env=self.env, source=source)

    def start(self):
        assert b"17.11" in self.command("postgres", "--version")
        self.command("initdb", "-D", self.data, "-U", "probe_owner", "--no-locale", "--encoding=UTF8",
                     "--auth-local=trust", "--auth-host=reject")
        with (self.data / "postgresql.conf").open("a") as config:
            config.write(f"""
listen_addresses = ''
unix_socket_directories = '{self.sock}'
unix_socket_permissions = 0700
logging_collector = on
log_destination = 'csvlog'
log_filename = 'screen.log'
log_rotation_age = 0
log_rotation_size = 0
shared_preload_libraries = 'auto_explain'
shared_buffers = '128MB'
work_mem = '4MB'
track_commit_timestamp = on
synchronous_commit = on
fsync = on
full_page_writes = on
autovacuum = off
timezone = 'UTC'
statement_timeout = '20s'
idle_in_transaction_session_timeout = '30s'
""")
        self.command("pg_ctl", "-D", self.data, "-w", "-t", 20, "-l", self.work / "startup.log", "start")
        self.sql("postgres", "CREATE ROLE parity_subject NOLOGIN; CREATE DATABASE base OWNER parity_subject;")
        self.sql("base", "CREATE EXTENSION pg_prewarm; SET ROLE parity_subject;\n" + SCHEMA.read_text())

    def stop(self):
        if (self.data / "postmaster.pid").exists():
            self.command("pg_ctl", "-D", self.data, "-m", "fast", "-w", "-t", 15, "stop")

    def psql(self, db):
        return [str(self.bindir / "psql"), "-X", "-w", "-v", "ON_ERROR_STOP=1", "-A", "-t", "-F", SEP, "-d", db]

    def sql(self, db, source, *, app="setup", variables=None):
        argv = self.psql(db)
        for key, value in (variables or {}).items():
            argv.extend(("-v", f"{key}={value}"))
        env = dict(self.env, PGAPPNAME=app)
        return checked(argv, env=env, source=(source + "\n").encode()).decode()

    def clone(self, target, source):
        self.sql("postgres", f"CREATE DATABASE {target} TEMPLATE {source};")

    def drop(self, db):
        self.sql("postgres", f"DROP DATABASE {db};")

    def warm(self, db):
        self.sql(db, """SELECT pg_prewarm(c.oid) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
                         WHERE n.nspname = 'mesh_jobs' AND c.relkind IN ('r','i') ORDER BY c.oid;""")


def load_harness():
    frozen = BENCH / "replay/frozen/scripts/jobs-parity.py"
    spec = importlib.util.spec_from_file_location("frozen_jobs_parity", frozen)
    harness = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(harness)
    return harness, json.loads(frozen.with_name("jobs-parity-profiles.json").read_text())


def load_inputs():
    harness, config = load_harness()
    inputs = {}
    for profile, digest in INPUT_DIGESTS.items():
        batches = harness.inputs(config["workloads"][profile], 1 if profile == "sparse" else 64, config["seed"])
        batch_hashes = [sha((json.dumps(batch, indent=2, sort_keys=True) + "\n").encode()) for batch in batches]
        assert sha("".join(batch_hashes).encode()) == digest, profile
        inputs[profile] = [job for batch in batches for job in batch["jobs"]]
    return inputs


def save_payload(path, jobs):
    with path.open("w") as stream:
        stream.write("COPY fixture_args FROM STDIN;\n")
        for n, job in enumerate(jobs, 1):
            stream.write(f"{n}\t{json.dumps(job)}\n")
        stream.write("\\.\n")


def wrapper():
    source = STORE.read_text()
    query, = re.findall(r'"(SELECT coalesce\(status, \'contention_hint\'\).*?\) END)"', source, re.S)
    types = "text, text, text, text, text, text, text"
    return f"PREPARE claim({types}) AS\n{query};\n"


def definition(variant):
    function, = re.findall(
        r"CREATE OR REPLACE FUNCTION mesh_jobs\.claim_batch_v1\(.*?\n\$\$;", SCHEMA.read_text(), re.S)
    cursor_tail = "        FOR NO KEY UPDATE OF j SKIP LOCKED;"
    assert function.count(cursor_tail) == 1
    return function.replace(cursor_tail, "        " + VARIANTS[variant] + cursor_tail.lstrip())


def start_locker(cluster, locked):
    app = dict(cluster.env, PGAPPNAME="locker")
    process = subprocess.Popen(cluster.psql("arm") + ["-q"], env=app, stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    positions = ",".join(map(str, locked))
    source = ("SET ROLE parity_subject; BEGIN ISOLATION LEVEL READ COMMITTED; "
              f"SELECT count(*) FROM (SELECT 1 FROM mesh_jobs.jobs WHERE position IN ({positions}) "
              "FOR NO KEY UPDATE) AS locked;\n")
    process.stdin.write(source.encode())
    process.stdin.flush()
    ready, _, _ = select.select([process.stdout], [], [], 20)
    if not ready or process.stdout.readline().decode().strip() != str(len(locked)):
        process.kill()
        process.communicate(timeout=5)
        raise RuntimeError("the independent locker did not confirm the requested locks")
    return process


def stop_locker(process):
    _, stderr = process.communicate(b"COMMIT;\n\\q\n", timeout=20)
    if process.returncode:
        raise RuntimeError(stderr.decode())


def claim(cluster, mode, capacity, slots, mixed, app):
    workers = [{"worker": "parity", "version": 1}]
    if mixed:
        workers += [{"worker": "alternate", "version": 2}, {"worker": "absent", "version": 1}]
    execute = f"EXECUTE claim('probe','default',{sql_string(json.dumps(workers))},'60000','{slots}','1700000005000','false');"
    # Auto is a reused backend. Five rolled-back calls populate the SPI cache
    # while leaving the sixth call responsible for the confirmed transitions.
    prime = ""
    if mode == "auto":
        prime = "\\o /dev/null\n" + ("SAVEPOINT warmup;\n" + execute +
                 "\nROLLBACK TO warmup;\nRELEASE warmup;\n") * 5 + "\\o\n"
    output = cluster.sql("arm", f"""
SET plan_cache_mode = '{mode}';
SET ROLE parity_subject;
{wrapper()}
BEGIN ISOLATION LEVEL READ COMMITTED;
{prime}
RESET ROLE;
{TRACE}
SET ROLE parity_subject;
\\echo row_start
{execute}
\\echo row_end
COMMIT;
""", app=app)
    _, rows = output.split("row_start\n")
    rows, commit = rows.split("row_end\n")
    assert commit == "COMMIT\n", commit
    reply, tokens = [], {}
    for line in rows.splitlines():
        status, id_, token, context, timeout, context_bytes, observed, ordinal = line.split(SEP)
        assert status == "claimed" and not observed and timeout == "60000"
        assert int(ordinal) == len(reply) + 1
        assert len(context.encode()) == int(context_bytes)
        body = json.loads(context)
        position = int(id_.replace("-", ""), 16)
        assert body["id"] == id_ and body["attempt"] == 1 and position not in tokens
        assert body["effect_key"] == f"{id_}/1"
        tokens[position] = token
        reply.append({"position": position, "context_bytes": int(context_bytes),
                      "context_sha256": sha(context.encode())})
    assert 1 <= len(reply) <= min(capacity, slots)
    rows = cluster.sql("arm", """SELECT position, lease_token, pg_xact_commit_timestamp(xmin) IS NOT NULL
                                 FROM mesh_jobs.jobs WHERE state = 'executing' ORDER BY position;""")
    persisted = {int(p): token for p, token, committed in (line.split(SEP) for line in rows.splitlines())
                 if committed == "t"}
    assert all(persisted.get(p) == token for p, token in tokens.items())
    return reply


def reduce_node(node):
    reduced = {key: node[key] for key in sorted(PATH_FIELDS & node.keys()) if node[key] != 0}
    # Preserve zero rows/loops explicitly in the sparse and locked cases.
    for key in ("Actual Rows", "Actual Loops"):
        if key in node:
            reduced[key] = node[key]
    if "Plans" in node:
        reduced["Plans"] = [reduce_node(child) for child in node["Plans"]]
    return reduced


def collect_plans(log):
    found = {}
    csv.field_size_limit(64 * 1024 * 1024)
    with log.open(newline="") as stream:
        for row in csv.reader(stream):
            message, application = row[13], row[22]
            if "plan:\n" not in message or not application.startswith("probe_"):
                continue
            explain_json = message.split("plan:\n", 1)[1]
            if not any(f'"Query Text": "{prefix}' in explain_json for prefix in
                       ("SELECT j AS job_row, j.xmin AS row_version", "PREPARE claim(")):
                continue
            plan = json.loads(explain_json)
            if isinstance(plan, list):
                plan, = plan
            if plan["Query Text"].startswith("SELECT j AS job_row, j.xmin AS row_version"):
                entry = found.setdefault(application, {})
                assert "cursor" not in entry, application
                entry.update({
                    "cursor": reduce_node(plan["Plan"]),
                    "cursor_sql_sha256": sha(plan["Query Text"].encode()),
                })
            elif plan["Query Text"].startswith("PREPARE claim("):
                found.setdefault(application, {})["outer"] = reduce_node(plan["Plan"])
    return found


def run(args, cluster):
    cluster.start()
    jobs = load_inputs()
    files = {}
    for profile, payload in jobs.items():
        files[profile] = cluster.work / (profile + ".sql")
        save_payload(files[profile], payload)
    settings = cluster.sql("base", """SELECT name, setting FROM pg_settings WHERE name IN (
    'server_version','work_mem','shared_buffers','cursor_tuple_fraction','fsync','full_page_writes',
    'synchronous_commit','track_commit_timestamp','server_encoding','autovacuum','random_page_cost')
    ORDER BY name;""")
    receipt = {
        "host": {"system": platform.system(), "architecture": platform.machine()},
        "settings": dict(line.split(SEP) for line in settings.splitlines()),
        "inputs_sha256": INPUT_DIGESTS, "schema_sha256": sha(SCHEMA.read_bytes()),
        "store_sha256": sha(STORE.read_bytes()), "cases": {},
    }
    assert receipt["settings"]["server_version"].split()[0] == "17.11"
    for case, profile, done, running, slots, locked, mixed in CASES:
        if case not in args.cases:
            continue
        capacity = 1 if case == "sparse" else 64
        cluster.clone("fixture", "base")
        cluster.sql("fixture", FIXTURE.read_text(), variables={
            "payload_file": files[profile], "done": done, "running": running, "capacity": capacity,
            "is_sparse": str(profile == "sparse").lower(), "mixed": str(mixed).lower()})
        state_counts = cluster.sql("fixture", "SELECT state, count(*) FROM mesh_jobs.jobs GROUP BY state ORDER BY state;")
        snapshot = {"profile": profile, "slots": slots, "capacity": capacity, "blocked_locks": locked,
                    "state": {state: int(n) for state, n in (line.split(SEP) for line in state_counts.splitlines())},
                    "runs": {}}
        start = done + running + 1 + len(locked)
        expected = [14, 15, 19] if mixed else list(range(start, min(start + slots, len(jobs[profile]) + 1)))
        for mode in args.modes:
            for variant in args.variants:
                cluster.clone("arm", "fixture")
                if variant != "stock":
                    cluster.sql("arm", "SET ROLE parity_subject;\n" + definition(variant))
                cluster.warm("arm")
                locker = start_locker(cluster, locked) if locked else None
                name = f"probe_{case}_{mode}_{variant}"
                try:
                    response = claim(cluster, mode, capacity, slots, mixed, name)
                    assert [r["position"] for r in response] == expected, (name, response, expected)
                finally:
                    if locker:
                        stop_locker(locker)
                cluster.drop("arm")
                snapshot["runs"][name] = {"reply": response, "commit_confirmed": True}
        cluster.drop("fixture")
        replies = [row["reply"] for row in snapshot["runs"].values()]
        assert all(reply == replies[0] for reply in replies), case
        receipt["cases"][case] = snapshot
        print(f"{case}: {len(replies)} confirmed claims", flush=True)
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pg-bin", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=ROOT / "target/jobs-claim-cursor-plan")
    parser.add_argument("--cases", choices=[case[0] for case in CASES], nargs="+", default=[case[0] for case in CASES])
    parser.add_argument("--modes", choices=MODES, nargs="+", default=MODES)
    parser.add_argument("--variants", choices=list(VARIANTS), nargs="+", default=list(VARIANTS))
    args = parser.parse_args()
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    # macOS Unix socket paths are shorter than ordinary filesystem paths.
    with (tempfile.TemporaryDirectory(dir=args.output, prefix="p") as directory,
          tempfile.TemporaryDirectory(dir=ROOT / "target", prefix=".claim-s-") as socket_dir):
        cluster = Cluster(Path(directory), Path(socket_dir), args.pg_bin.resolve())
        try:
            receipt = run(args, cluster)
        finally:
            cluster.stop()
        log = cluster.data / "log/screen.csv"
        assert log.is_file(), log
        plans = collect_plans(log)
        for snapshot in receipt["cases"].values():
            for name, run_receipt in snapshot["runs"].items():
                assert {"cursor", "outer"} <= plans[name].keys(), name
                run_receipt.update(plans[name])
        (args.output / "plans.json").write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
