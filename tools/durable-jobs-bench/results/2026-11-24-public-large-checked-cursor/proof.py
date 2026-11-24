"""Untimed tests of the sealed public runner and isolated SQL screen."""
import argparse
import json
import os
from pathlib import Path
import sys
from types import ModuleType

from audit import ROOT, REVISION, digest, read, statement
from diagnostic import install_thin, physical

PRODUCT = ROOT / "target/2026-11-24-trusted-jobs-product-composition"
REPLAY = PRODUCT / "source/tools/durable-jobs-bench/results/2026-11-24-trusted-jobs-product-composition/replay"


def relay_class():
    source = (REPLAY / "wire.py").read_text()
    # Extend the sealed relay to observe checked cursor grants as well as set
    # grants, and to hold/drop SELECT's transactional ReadyForQuery.
    before = 'if kind == b"D" and b"FROM mesh_jobs.claim_batch_set_v1(" in query:'
    after = '''if kind == b"D" and any(fn in query for fn in
                    (b"FROM mesh_jobs.claim_batch_set_v1(", b"FROM mesh_jobs.claim_batch_v1(")):'''
    assert source.count(before) == 1
    source = source.replace(before, after)
    before = 'if data == b"I" and command:'
    after = 'if (data == b"I" or (data == b"T" and grant_rows)) and command:'
    assert source.count(before) == 1
    source = source.replace(before, after)
    before = 'rows=count, idle=True,'
    assert source.count(before) == 1
    source = source.replace(before, 'rows=count, idle=data == b"I",')
    module = ModuleType("cursor_proof_wire")
    sys.modules[module.__name__] = module
    exec(compile(source, str(REPLAY / "wire.py"), "exec"), module.__dict__)
    return module.Relay


def check(out, executable, storage, mode):
    import semantic as sem

    os.environ.update(PRODUCT_COMPOSITION="product-bulk", PRODUCT_STORAGE=storage, PRODUCT_TOPOLOGY="stock")
    arm = sem.arm
    with sem.cluster(out, executable, "fault") as (db, subject, env), relay_class()(db) as relay:
        pg = lambda q: db.sql("mesh", q)
        shim = install_thin(pg, sem.NS) if mode == "thin" else None
        fault = mode in ("rollback", "commit-loss")
        needle = b"COMMIT" if mode == "commit-loss" else b"FROM mesh_jobs.claim_batch_v1("
        gate = relay.arm(needle, drop=fault)
        small_gate = None
        log = out / "runner.log"
        proc = subject.run(dict(env, JOBS_PARITY_POOL_URL=db.url("mesh", port=relay.port)), 0, log)
        try:
            sem.wait("runner ready", lambda: arm.h.runner_ready([proc], [log], track=False))
            big = sem.submit(out, subject, env, "large-first", [("large", {"payload": "a" * 524288})])
            assert gate.hit.wait(6), "no checked cursor receipt"
            held = sem.rows(db)["large"]
            committed = mode == "commit-loss"
            assert (held["state"], held["attempt"]) == (("executing", 1) if committed else ("available", 0))
            assert held["effect"] is None and not held["attempts"]
            assert gate.witness == dict(command="COMMIT" if committed else "SELECT 1", rows=0 if committed else 1,
                                        idle=committed, receipt_dropped=fault)
            gate.release.set()
            sem.wait("large settlement", lambda: sem.rows(db)["large"]["state"] == "completed")
            snapshot = sem.rows(db)["large"]
            assert snapshot["attempt"] == (2 if committed else 1) and len(snapshot["attempts"]) == 1
            assert len({big["xid"], snapshot["attempts"][0], snapshot["effect"], snapshot["xid"]}) == 4
            pg_row, = physical(pg, sem.NS)
            assert pg_row["payload_bytes"] == 524288
            if storage == "default":
                assert pg_row["args_compression"] is not None
            else:
                assert pg_row["args_compression"] is None and pg_row["size_sum"] > 65536
            if committed:
                app_commit = int(pg(f"""SELECT floor(extract(epoch FROM pg_xact_commit_timestamp(xmin))*1000)::bigint
                    FROM public.mesh_jobs_parity_attempts WHERE namespace='{sem.NS}' AND job_key='large'"""))
                assert app_commit >= held["expiry"]
            # One connection must show the exact set/commit/probe/cursor chain.
            chains = {}
            for port, q in list(relay.requests):
                label = statement(q)
                if label:
                    chains.setdefault(port, []).append(label)
            sequence = ["begin", "set", "commit", "probe_due", "begin", "cursor"]
            assert any(sequence == x[i:i+len(sequence)] for x in chains.values() for i in range(len(x))), chains
            if mode != "thin":
                small_gate = relay.arm(b"FROM mesh_jobs.claim_batch_set_v1(")
                small = sem.submit(out, subject, env, "small-next", [("small", {})])
                assert small_gate.hit.wait(6), "no checked set grant after draining large work"
                assert small_gate.witness == dict(command="SELECT 1", rows=1, idle=False, receipt_dropped=False)
                assert sem.rows(db)["small"]["state"] == "available"
                small_gate.release.set()
                sem.wait("small settlement", lambda: sem.rows(db)["small"]["state"] == "completed")
                snapshot_small = sem.rows(db)["small"]
                assert len({small["xid"], snapshot_small["attempts"][0], snapshot_small["effect"], snapshot_small["xid"]}) == 4
            else:
                snapshot_small = None
            arm.h.stop_runners([proc], require_clean=True)
            assert not relay.errors
            arm.h.write_json(out / "result.json", dict(mode=mode, physical=pg_row,
                             held=held, final_large=snapshot, final_small=snapshot_small,
                             wire=gate.witness, observed_chain=sequence, shim=shim,
                             set_wire=small_gate.witness if small_gate else None))
        finally:
            gate.release.set()
            if small_gate:
                small_gate.release.set()
            if proc.poll() is None:
                arm.h.stop_runners([proc], require_clean=False)


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("output", type=Path)
    cli.add_argument("mode", choices=("all", "hold", "thin", "rollback", "commit-loss"), default="all", nargs="?")
    args = cli.parse_args()
    out = args.output.resolve()
    assert out.is_relative_to(ROOT / "target")
    out.mkdir()
    exe = PRODUCT / "local-fixture-sealed/mesh"
    build = read(exe.with_suffix(".json"))
    assert build["version"] == REVISION and digest(exe) == build["files"]["mesh"]
    os.environ["PRODUCT_SOURCE"] = str(PRODUCT / "source")
    os.environ.setdefault("PRODUCT_FROZEN_HARNESS", str(ROOT / "target/short-exit-bulk-d2/source"))
    sys.path.insert(0, str(REPLAY))
    cases = (("default", "hold"), ("external-4m", "hold"), ("default", "thin"),
             ("external-4m", "thin"), ("default", "rollback"), ("default", "commit-loss"))
    for storage, mode in cases:
        if args.mode in ("all", mode):
            check(out / f"{storage}-{mode}", exe, storage, mode)
            print(f"PASS {storage} {mode}", flush=True)


if __name__ == "__main__":
    main()
