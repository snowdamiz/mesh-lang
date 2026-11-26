"""Replay Jobs ABI calls against one controller-owned PG17 cluster."""
import hashlib
import json
import os
from pathlib import Path
import pwd
import shutil
import socket
import subprocess
import sys

from jobs_parity_fault import SettlementProxy

LANES = ("stock", "move", "candidate")
def object_bytes(value):
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False).encode()


def main(pg, home, work):
    work.mkdir()
    data = work / "postgres"
    env = dict(os.environ, TMPDIR=str(work))
    user = pwd.getpwuid(os.getuid()).pw_name
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    with (work / "initdb.log").open("xb") as log:
        subprocess.run([pg / "initdb", "-D", data, "--auth=trust", "--encoding=UTF8", "--no-locale"],
                       env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    with (data / "postgresql.conf").open("a") as settings:
        settings.write(f"""
listen_addresses = '127.0.0.1'
unix_socket_directories = ''
port = {port}
fsync = on
synchronous_commit = on
shared_preload_libraries = 'pg_stat_statements'
track_commit_timestamp = on
""")
    pgctl = pg / "pg_ctl"
    started = False
    report = {}
    calls = []

    def sql(query):
        return subprocess.check_output(
            [pg / "psql", "-XAt", "-v", "ON_ERROR_STOP=1", "-h", "127.0.0.1",
             "-p", str(port), "-U", user, "-d", "postgres", "-c", query], env=env, text=True).strip()

    def call(lane, ns, name, raw=b"", *, mode="enqueue", use_port=port, bulk=0):
        path = work / f"input-{name}.json"
        if mode != "migrate":
            path.write_bytes(raw)
        result = subprocess.run([home / "bin" / lane, "wire", mode, ns, path], check=True, timeout=45,
                                capture_output=True,
                                env=dict(env, MESH_JOBS_SET_BASED_ENQUEUE=str(bulk),
                                         ENQUEUE_SPEC_TEST_PG_URL=f"postgresql://{user}@127.0.0.1:{use_port}/postgres?sslmode=disable"))
        assert not result.stderr, result.stderr
        row = json.loads(result.stdout)
        calls.append({"lane":lane, "namespace":ns, "case":name, "mode":mode, "bulk":bulk,
                      "input_sha256":hashlib.sha256(raw).hexdigest(), "result":row})
        return row

    def ledger(ns):
        return json.loads(sql(f"""
SELECT json_build_object(
 'jobs', (SELECT count(*) FROM mesh_jobs.jobs WHERE namespace='{ns}'),
 'receipts', (SELECT coalesce(json_agg(json_build_object('key',request_key,'fingerprint',fingerprint,'receipt',receipt)
                ORDER BY request_key),'[]') FROM mesh_jobs.receipts WHERE namespace='{ns}'),
 'admissions', (SELECT count(*) FROM mesh_jobs.admissions WHERE namespace='{ns}'),
 'queues', (SELECT count(*) FROM mesh_jobs.queues WHERE namespace='{ns}'))"""))

    def empty(ns):
        assert ledger(ns) == {"jobs":0, "receipts":[], "admissions":0, "queues":0}

    try:
        subprocess.run([pgctl, "start", "-D", data, "-l", work / "postgres.log", "-w"],
                       env=env, stdout=subprocess.DEVNULL, check=True)
        started = True
        assert sql("SHOW server_version_num") == "170011"
        assert sql("SHOW fsync") == sql("SHOW synchronous_commit") == "on"
        assert call("stock", "proof", "schema", mode="migrate") == {"ok":True}
        sql("CREATE EXTENSION pg_stat_statements")
        sql("""
CREATE FUNCTION public.reject_candidate_commit() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.namespace LIKE 'commit-%' THEN RAISE EXCEPTION 'qualification deferred COMMIT fault'; END IF;
  RETURN NEW;
END $$;
CREATE CONSTRAINT TRIGGER qualification_commit
AFTER INSERT ON mesh_jobs.jobs DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION public.reject_candidate_commit();
""")
        fixture = (home / "frames/large/0").read_bytes()
        frame, = json.loads(fixture)
        escaped = bytearray(fixture)
        needle = b'"payload":"'
        end = fixture.index(needle) + len(needle) + 524_287
        escaped[end:end+1] = b"\\u001f"
        escaped = bytes(escaped)
        errors = {}
        for index, lane in enumerate(LANES):
            for kind, raw in (("plain", fixture), ("escaped", escaped)):
                ns = f"retry-{kind}-{index}"
                first = call(lane, ns, "first", raw)["ok"]
                rid = first[0]["id"]
                assert first[0]["effect_key"] == f"{rid}/1"
                observed = ledger(ns)
                assert observed["jobs"] == observed["admissions"] == observed["queues"] == 1
                receipt, = observed["receipts"]
                assert receipt["key"] == frame["request_key"] and receipt["receipt"] == first[0]
                assert len(receipt["fingerprint"]) == 64
                assert rid == sql(f"SELECT id FROM mesh_jobs.jobs WHERE namespace='{ns}'")
                persisted, = json.loads(raw)
                expected = hashlib.md5(persisted["args"]["payload"].encode()).hexdigest()
                assert sql(f"SELECT md5(args->>'payload') FROM mesh_jobs.jobs WHERE namespace='{ns}'") == expected
                for retry in LANES:
                    assert call(retry, ns, "same-key", raw) == {"ok":first}
                    changed = bytearray(raw)
                    payload_start = raw.index(needle) + len(needle)
                    changed[payload_start] = ord("Q") if changed[payload_start] != ord("Q") else ord("V")
                    assert "already bound" in call(retry, ns, "changed-key", bytes(changed))["error"]
                assert ledger(ns) == observed
                report[ns] = observed
            invalid = object_bytes([frame, {"worker":"deliver","args":{"bad":"\0"}}])
            ns = f"sql-{index}"
            errors.setdefault("sql", []).append(call(lane, ns, "sql-fault", invalid)["error"])
            errors.setdefault("savepoint", []).append(call(lane, ns, "sql-outer", invalid, mode="outer")["error"])
            assert "Unicode escape" in errors["sql"][-1]
            empty(ns)
            assert "ok" in call(lane, ns, "outer-rollback", fixture, mode="outer")
            empty(ns)
            ns = f"commit-{index}"
            errors.setdefault("commit", []).append(call(lane, ns, "deferred-commit", fixture)["error"])
            assert "qualification deferred COMMIT fault" in errors["commit"][-1]
            empty(ns)
            ns = f"utf8-{index}"
            assert call(lane, ns, "bad-utf8", b'{"worker":"\xff"}') == {"invalid_utf8":True}
            empty(ns)
            for placement in ("before_commit", "after_commit"):
                ns = f"lost-{placement.replace('_', '-')}-{index}"
                with SettlementProxy(port, lambda text: text.strip().upper() == "COMMIT", placement) as proxy:
                    result = call(lane, ns, placement, fixture, use_port=proxy.port)
                assert "error" in result and proxy.fault, (result, proxy.fault)
                observed = ledger(ns)
                if placement == "before_commit":
                    empty(ns)
                else:
                    assert observed["jobs"] == len(observed["receipts"]) == 1
                first = call("stock", ns, "reconcile", fixture)["ok"]
                assert ledger(ns)["jobs"] == 1
                if placement == "after_commit":
                    assert first == [observed["receipts"][0]["receipt"]]
                    assert ledger(ns) == observed
                report[ns] = {"fault":proxy.fault, "at_fault":observed, "reconciled":ledger(ns)}
            ns = f"bulk-{index}"
            batch = object_bytes([{"worker":"deliver", "args":{"n":n},
                                   "request_key":f"bulk-{n}"} for n in range(16)])
            first = call(lane, ns, "bulk-first", batch, bulk=1)["ok"]
            assert len(first) == ledger(ns)["jobs"] == 16
            assert call("stock", ns, "bulk-retry", batch, bulk=1) == {"ok":first}
        for values in errors.values():
            assert len(set(values)) == 1, values
        for kind in ("plain", "escaped"):
            assert len({report[f"retry-{kind}-{i}"]["receipts"][0]["fingerprint"] for i in range(3)}) == 1
        bulk = sql("""SELECT coalesce(sum(calls),0) FROM pg_stat_statements
WHERE query LIKE 'SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(%'""")
        assert int(bulk) >= 6
        report["set_based_calls"] = int(bulk)
        report["postgres"] = sql("SHOW server_version")
        (work / "wire.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    finally:
        (work / "calls.json").write_text(json.dumps(calls, indent=2, sort_keys=True) + "\n")
        if started:
            subprocess.run([pgctl, "stop", "-m", "fast", "-w", "-D", data], env=env,
                           stdout=subprocess.DEVNULL, check=True)
        shutil.rmtree(data)


if __name__ == "__main__":
    main(*(Path(arg).resolve() for arg in sys.argv[1:]))
