#!/usr/bin/env python3
"""Reconcile the preserved Linux SETUP receipts and verify their four-way pins."""

import json
import os
from pathlib import Path
import re
import shlex
import subprocess
from types import ModuleType

from pack import HERE, NATIVE, audit, audit_overlays, members, require, sha

ROOT = HERE.parents[3]
SUITE = "tools/durable-jobs-bench"
RESULTS = HERE.parent
PRODUCT = "3f69b150820fd01ba716d533be4f2eedea25ed27"
FIXTURE = "dbcb6dbc1de3a6887fc3757c2b91f7974d74ed17"
VERIFIER = "1406e449cf2f24f3363369d8cb3bff33c14ba7c6"
VERIFIER_SHA256 = "5e5c06284adbeaaeb71708c2ccf885dd7bf3b4783f09515885e427ffbdfc7c2f"
INVENTORY_SHA256 = "3ce57d42b48e70262aa548162064f11c655641dda86a6b1e037b55de97dadcbc"

FIXTURE_FILES = (
    "scripts/jobs-parity.py", "scripts/jobs-parity-profiles.json",
    "scripts/ci-x86_64-linux-gnu-clang",
    *(f"scripts/fixtures/jobs-parity-oban/{name}" for name in (
        "mix.exs", "mix.lock", "inetrc", "config/config.exs",
    )),
    "proof/jobs-parity/linux-warm-replay-20261122.tar.gz",
    *(f"proof/jobs-parity/linux-regression/{name}" for name in (
        "trial.py", "proof_transport.py", "oban_runtime.py", "application.sql",
        "fence.sql", "fixture.exs", "oban-session.ex.in",
    )),
)


def sums(data):
    return dict((name, digest) for digest, name in (
        line.split("  ", 1) for line in data.decode().splitlines()
    ))


def read(name):
    return (HERE / name).read_bytes()


def git(*args):
    return subprocess.check_output(["git", "-C", str(ROOT), *args])


def verify_source():
    expected = sums(read("source-v5.sha256"))
    overlays = members(read("staged-source-overlays.tar.gz"))
    audit_overlays(overlays)
    source = members(git("archive", PRODUCT, "Cargo.toml", "Cargo.lock", ".cargo/config.toml",
                         "compiler", "tools/skill/mesh"))
    fixture = members(git("archive", FIXTURE, *FIXTURE_FILES))
    source.update(fixture)
    source["NATIVE_MESH_REVISION"] = (PRODUCT + "\n").encode()
    for path in expected:
        if path.startswith(NATIVE + "/"):
            source[path] = (overlays["v5/README.md"] if Path(path).name == "README.md"
                            else (ROOT / path).read_bytes())
    require(expected == {name: sha(data) for name, data in source.items()}, "source-v5 snapshot drift")
    for path in ("bootstrap", "build", "linode", "run"):
        require(os.access(ROOT / NATIVE / path, os.X_OK), f"native launcher is not executable: {path}")

    # The pinned verifier resolves the frozen input generator from the checkout.
    for path in ("scripts/jobs-parity.py", "scripts/jobs-parity-profiles.json"):
        require((ROOT / path).read_bytes() == fixture[path], f"frozen input source drift: {path}")
    return expected


def load_verifier():
    path = f"{SUITE}/verify.py"
    code = git("show", f"{VERIFIER}:{path}")
    require(sha(code) == VERIFIER_SHA256, "pinned matrix verifier digest differs")
    module = ModuleType("mesh_oban_matrix_verifier")
    module.__file__ = str(ROOT / path)
    exec(compile(code, module.__file__, "exec"), module.__dict__)
    return module


def verify_four_way(staged, ready):
    raw = (RESULTS / "2026-11-23-baseline/inventory.json").read_bytes()
    require(sha(raw) == INVENTORY_SHA256, "official baseline inventory drift")
    inventory = json.loads(raw)
    require(ready["cpus"] == inventory["cpus"] == [0, 1], "setup and baseline CPU pin differs")
    engines = inventory["engines"]
    require(set(engines) == {"mesh", "oban", "go", "rust"}, "expected four baseline engines")
    for engine, entry in ready["engines"].items():
        require(entry == engines[engine], f"official {engine} entry differs from READY")
        for path, digest in entry["verified_files"].items():
            require(staged[path.split("/source-v5/", 1)[1]] == digest, f"staged source drift: {path}")
        require(sha(read(f"{engine}.json")) == entry["artifact_sha256"],
                f"{engine} build manifest hash differs")

    river_dir = RESULTS / "2026-11-23-river-submit"
    river = json.loads((river_dir / "READY.json").read_bytes())
    require(all(river[key] == value for key, value in engines["go"].items()), "River READY differs")
    river_wrapper = (river_dir / "wrapper-source.tar.gz").read_bytes()
    require(sha(river_wrapper) == river["wrapper_source_sha256"], "River wrapper archive digest differs")
    wrapper_files = members(river_wrapper)
    for remote, digest in river["verified_files"].items():
        if remote.endswith("/river-bench"):
            actual = river["artifact_sha256"]
        elif remote.endswith("/wrapper-source.tar.gz"):
            actual = sha(river_wrapper)
        else:
            path = remote.split("/source/", 1)[1]
            actual = sha(wrapper_files[path])
            if path != "SHA256SUMS":
                require(sha((ROOT / path).read_bytes()) == digest, f"River checkout drift: {path}")
        require(actual == digest, f"River staged digest differs: {remote}")

    apalis_dir = RESULTS / "2026-11-23-apalis-smoke"
    apalis = json.loads((RESULTS / "2026-11-23-native/apalis.json").read_bytes())
    rust = engines["rust"]
    require(all(rust[key] == value for key, value in apalis["artifact"].items()), "Apalis artifact differs")
    for path, digest in apalis["build"]["source_sha256"].items():
        require(sha((ROOT / SUITE / "rust" / path).read_bytes()) == digest, f"Apalis source drift: {path}")
    for remote, digest in rust["verified_files"].items():
        require(apalis["build"]["source_sha256"][Path(remote).name] == digest,
                f"Apalis staged digest differs: {remote}")
    apalis_archive = members((apalis_dir / "sparse-trace-off-v1.tar.gz").read_bytes())
    command = shlex.split(apalis_archive["command.txt"].decode())
    require(command[:3] == ["runuser", "-u", rust["run_as"]] and command[command.index(rust["argv"][0]):-2] == rust["argv"],
            "Apalis launcher differs from the archived smoke")
    require(sha(apalis_archive["adapter/build.json"]) == apalis["build_manifest_sha256"],
            "Apalis build manifest digest differs")
    return {engine: entry["artifact_sha256"] for engine, entry in engines.items()}


def verify_arm(files, row, ready, verifier):
    engine, profile = row["engine"], row["profile"]
    manifest = json.loads(read(f"{engine}.json"))
    artifact = {key: ready["engines"][engine][key] for key in ("version", "artifact_sha256")}

    def load(name):
        return json.loads(files[name + ".json"])

    invocation = dict(engine=engine, profile=profile, phase="diagnostic", local_smoke=False)
    ledger, observed, host = load("ledger"), load("row"), load("host-before")
    require(invocation == load("invocation") and all(
        ledger[key] == observed[key] == value for key, value in invocation.items()
    ), "expected native diagnostic invocation")
    require((host["system"], host["architecture"], host["host_logical_cpus"], host["cpus"])
            == ("Linux", "x86_64", 4, [0, 1]), "wrong Linux host or runner affinity")
    require((manifest["system"], manifest["architecture"], manifest["version"])
            == ("Linux", "x86_64", artifact["version"]), "wrong native build identity")
    require(all(observed[k] == v for k, v in row.items() if k in observed), "owner row differs from arm")

    # Dense ran the no-op frozen input under its own diagnostic label.
    case = "noop" if profile == "dense" else profile
    metrics = verifier.reconcile(dict(ledger, profile=case), engine, case, artifact)
    require(ledger["database"] == load("postgres-settings"), "database settings differ from ledger")
    affinity = load("process-affinity")
    require(len(affinity) == ledger["runners"] + 1 and all(mask == ready["cpus"] for mask in affinity.values()),
            "runner or PostgreSQL postmaster was not pinned")
    roles = {"parity_producer", "parity_subject"}
    if engine == "mesh":
        roles.add("compiled_caller")
    else:
        require(json.loads(files["oban-versions.log"].splitlines()[-1])
                == {"oban": "2.24.2", "elixir": "1.20.4", "erlang": "29.1.1"},
                "unexpected live Oban/BEAM versions")
    sessions = load("postgres-tls-sessions")
    require({s["role"] for s in sessions} == roles and all(s["tls"] is True for s in sessions),
            "missing TLS session evidence")

    records = load("application-records")
    jobs = ledger["jobs"]
    for file_key, job_key in (("attempts", "attempt_xid"), ("effects", "effect_xid")):
        writes = records[file_key]
        require(len(writes) == len(jobs) and all(write["engine"] == engine for write in writes)
                and {(write["job_key"], write["xid"]) for write in writes}
                == {(job["job_key"], job[job_key]) for job in jobs}, f"independent {file_key} differ")
    for transaction in ledger["transactions"]:
        require(transaction["submitted_us"] <= transaction["committed_us"] <= transaction["acknowledged_us"],
                "commit outside the producer call window")
    completed = metrics["completed"]
    require(completed == metrics["ordered_independent_writes"] == row["reconciled_jobs"]
            and 2 * completed == row["committed_work_records"]
            and len(ledger["transactions"]) == row["durable_enqueue_transactions"],
            "owner row counts differ from replay")
    return {"jobs": completed, "ordered_application_commits": 2 * completed,
            "enqueue_transactions": len(ledger["transactions"])}


def verify_smokes(index, ready, verifier):
    files = members(read("native-smokes.tar.gz"))
    audit(files)
    require({name: sha(data) for name, data in files.items()} == index["retained_smoke_files"],
            "retained smoke member digest differs")
    rows = {Path(row["evidence"]).parent.name: row for row in index["original_rows"]}
    expected = {(engine, profile) for engine in ("mesh", "oban")
                for profile in (*verifier.PROFILES, "dense")}
    require(len(rows) == len(expected) and {(row["engine"], row["profile"]) for row in rows.values()} == expected,
            "missing or duplicate diagnostic profiles")
    successes, failures = {}, {}
    for name, receipt in index["attempts"].items():
        arm = {path.split("/", 1)[1]: data for path, data in files.items() if path.startswith(name + "/")}
        status = int(arm["exit-status.txt"])
        require(status == receipt["exit_status"], f"exit status differs: {name}")
        if status == 0:
            require(rows[name]["evidence_sha256"] == receipt["original_sha256"],
                    f"original receipt digest differs: {name}")
            successes[name] = verify_arm(arm, rows[name], ready, verifier)
        else:
            require(status == 1 and name not in rows and "ledger.json" not in arm,
                    f"unexpected failed attempt: {name}")
            if name == "oban-short-01":
                require(b"Kernel pid terminated (logger)" in arm["failure.json"]
                        and b"warm_producer" in arm["failure.json"], "missing original broker startup failure")
                failures[name] = "broker startup"
            else:
                require(set(arm) == {"exit-status.txt", "run.log", "export.log"}, "expected early refusal")
                failures[name] = "exited before startup"
    require(successes.keys() == rows.keys()
            and failures.keys() == {"oban-short-01", "oban-dense-01", "oban-dense-02"},
            "missing native attempts")
    return successes, failures


def verify_port(index):
    archives = {}
    for name in ("native-proof-logs", "native-wait-trace"):
        raw = read(name + ".tar.gz")
        require(sha(raw) == index["original_archives"][name], f"original digest differs: {name}")
        archives[name] = members(raw)
        audit(archives[name])
    logs, trace = archives["native-proof-logs"], archives["native-wait-trace"]
    for name, code in {
        "effects-poll": 0, "compiled-poll-fresh": 0, "compiled-listen-thp-fresh": 0,
        "bounded-json": 0, "session-reset": 0, "effect-fence": 0, "pg-17.11-compat": 0,
        "baseline-host-gate": 2, "baseline-exact-build": 0, "baseline-exact-refusal": 1,
    }.items():
        require(int(logs[f"logs/{name}.result"]) == code, f"unexpected proof status: {name}")
    require(b"--effect-compiled requires Linux GNU ARM64." in logs["logs/baseline-host-gate.log"]
            and b"Jobs effects require a compiled Linux GNU ARM64 Mesh program"
            in logs["logs/baseline-exact-refusal.log"], "missing historical AMD64 refusal")
    for mode in ("effects-poll", "compiled-poll-fresh", "compiled-listen-thp-fresh"):
        log = logs[f"logs/{mode}.log"]
        for witness in (b"same verified warm backend, two commits", b"reaped the held child",
                        b"two durable effects after a replacement attempt", b"control EOF stopped the exact held child"):
            require(witness in log, f"missing compiled {mode} witness")
    compat = logs["logs/pg-17.11-compat.log"]
    for witness in (b"PostgreSQL 17.11 over TLSv1.3", b"server rejected a wrong password",
                    b"85 passed; 0 failed", b"3 passed; 0 failed"):
        require(witness in compat, "missing PostgreSQL compatibility witness")

    line, *exits = read("exact-wait.txt").splitlines()
    wait = re.fullmatch(rb"Runner wait thread (\d+) consumed worker (\d+): (wait4\(.*)", line)
    require(wait is not None, "missing consuming wait4 witness")
    waiter, worker, call = wait.groups()
    require(call == b"wait4(" + worker + b", [{WIFEXITED(s) && WEXITSTATUS(s) == 171}], WNOHANG, NULL) = " + worker
            and call in trace["trace/native." + waiter.decode()].splitlines()
            and b"+++ exited with 171 +++" in trace["trace/native." + worker.decode()],
            "timeout wait4 did not consume the exact child")
    for record in exits:
        thread, exit_call = record.removeprefix(b"Thread ").split(b": ", 1)
        require(exit_call in trace["trace/native." + thread.decode()].splitlines(),
                "exit_group witness missing from trace")
    require(any(b"exit_group(173)" in line for line in exits), "missing parent EOF exit")
    return {"historical_host_gate": 2, "historical_compiled_refusal": 1,
            "timeout_wait_status": 171, "parent_eof_exit_status": 173,
            "fresh_linux_proofs": ["poll", "listen_thp", "postgres_17.11_compatibility"]}


def main():
    for name, digest in sums(read("SHA256SUMS")).items():
        require(sha(read(name)) == digest, f"bundle checksum differs: {name}")
    audit(members(read("setup-history.tar.gz")))
    index, ready = json.loads(read("originals.json")), json.loads(read("READY.json"))
    engines = verify_four_way(verify_source(), ready)
    arms, failures = verify_smokes(index, ready, load_verifier())
    proof = verify_port(index)
    print(json.dumps({
        "phase": "SETUP", "matrix_samples": 0, "baseline_artifact_sha256": engines,
        "smokes": arms, "failed_attempts": failures, "amd64_port": proof,
        "total_jobs": sum(row["jobs"] for row in arms.values()),
        "total_application_commits": sum(row["ordered_application_commits"] for row in arms.values()),
    }, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
