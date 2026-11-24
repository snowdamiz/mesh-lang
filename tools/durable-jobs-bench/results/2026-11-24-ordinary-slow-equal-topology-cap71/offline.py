"""Replay physical budget witnesses from the sealed dense and slow archives."""

from collections import Counter
from pathlib import Path
import argparse
import json
import tarfile

from qualification import CAP, PACKET, PRIOR, ROOT, hold_peak, require, run, physical

DENSE = PACKET.parent / "2026-11-23-dense-matched-topology"
DEPS = ROOT / "target/linode-river/cache/mod/github.com"
APALIS = ROOT / "target/linode-apalis"


def archive_sha(manifest, key):
    matches = [digest for digest, path in (line.split(maxsplit=1) for line in manifest.read_text().splitlines())
               if path.removeprefix("./") == key]
    require(len(matches) == 1, f"no unique archive seal {key}")
    return matches[0]


def read_witness(path, engine, cap, manifest, key):
    sha = run.digest(path)
    require(sha == archive_sha(manifest, key), f"archive checksum mismatch {path}")
    with tarfile.open(path) as archive:
        def read(name):
            return run.file_json(archive, "adapter/" + name)

        ledger = read("ledger.json")
        host = run.file_json(archive, "host.json")
        require(run.file_json(archive, "status.json")["exit_code"] == 0 and host["affinity"] == [0, 1]
                and ledger["database"]["max_connections"] == cap, "wrong archive, cap or affinity")
        roles = None
        pool_max = None
        if engine == "rust":
            ready = read("ready.json")["sessions"]
            roles = dict(Counter(row["application_name"] for row in ready if row["ssl"]))
            expected = {f"apalis:runner-{i}": 34 for i in range(2)} if cap == 96 else {"apalis:runner-0": 66}
            require(roles == {**expected, "apalis:producer_observer": 1}
                    and sum(row["listener"] for row in ready) == len(expected), "Apalis eager pool changed")
        if engine == "oban":
            rows = read("postgres-tls-sessions.json")
            roles = dict(Counter(row["role"] for row in rows if row["tls"]))
            require(roles == {"parity_producer": 1, "parity_subject": 68 if cap == 96 else 66},
                    "Oban warm roles changed")
        if engine == "go":
            cfg = read("river/config.json")
            pool_max = cfg["runner_pool_max"]
            require(pool_max == (68 if cap == 96 else 66) and cfg["producer_observer_pool_max"] == 1
                    and not cfg["poll_only"], "River pool or wakeup configuration changed")
            require(read("river/database.json")["other_client_backends"] == 0, "River preflight saw another client")
        return dict(archive=key, sha256=sha, cap=cap, jobs=len(ledger["jobs"]), runners=ledger["runners"],
                    slots=ledger["slots"], peak_connections=ledger["peak_connections"], pool_max=pool_max,
                    committed_hold_peak=hold_peak(ledger["jobs"]), warm_roles=roles)


def native_receipts():
    dense, slow = {}, {}
    series = DENSE / "evidence/equal"
    for block in sorted((series / "blocks").glob("*")):
        if not (block / "reconciled.json").is_file():
            continue
        for name, engine in (("apalis", "rust"), ("river", "go"), ("oban", "oban")):
            raw, = block.glob(f"measured-{name}-2x32/*/raw.tar.gz")
            key = str(raw.relative_to(series))
            dense[f"{block.name}/{name}"] = read_witness(raw, engine, 96, series / "SHA256SUMS", key)
    series = PRIOR / "evidence/native-import-retry"
    for block in sorted((series / "blocks").glob("*")):
        if not (block / "reconciled.json").is_file():
            continue
        for name, engine in (("apalis-stock-1x64", "rust"), ("river-stock-1x64", "go"),
                             ("oban-stock-1x64", "oban"), ("two", "mesh")):
            raw, = block.glob(f"measured-{name}/*/raw.tar.gz")
            key = str(raw.relative_to(series))
            slow[f"{block.name}/{name}"] = read_witness(raw, engine, 68, series / "SHA256SUMS", key)
    peaks = {name: sorted(row["peak_connections"] for key, row in dense.items() if key.endswith("/" + name))
             for name in ("apalis", "oban", "river")}
    require(len(dense) == 15 and len(slow) == 8 and peaks ==
            {"apalis": [69]*5, "oban": [69]*5, "river": [67, 67, 68, 68, 70]},
            "native budget witnesses changed")
    require(all(row["committed_hold_peak"] == 64 for row in slow.values()),
            "canonical slow capacity was not observed")
    return dict(native_equal_dense=dense, archived_slow_cap68=slow, dense_peaks=peaks)


def source_receipts():
    files = (
        (ROOT / "tools/durable-jobs-bench/go/worker.go", ("w.pool.Acquire(ctx)", "int32(slots+2)", "defer conn.Release()")),
        (ROOT / "tools/durable-jobs-bench/go/db.go", ("cfg.MaxConns, cfg.MinConns, cfg.MinIdleConns = cap, 0, 0",)),
        (ROOT / "tools/durable-jobs-bench/go/main.go", ('"river:producer_observer", 1',)),
        (DEPS / "riverqueue/river/riverdriver/riverpgxv5@v0.51.2/river_pgx_v5_driver.go",
         ("poolConn, err := l.dbPool.Acquire(ctx)", "l.conn = poolConn.Hijack()")),
        (DEPS / "jackc/pgx/v5@v5.13.0/pgxpool/conn.go", ("res.Hijack()",)),
        (DEPS / "jackc/puddle/v2@v2.2.3/pool.go",
         ("func (p *Pool[T]) hijackAcquiredResource", "p.allResources.remove(res)", "defer p.acquireSem.Release(1)")),
        (ROOT / "tools/durable-jobs-bench/rust/src/main.rs",
         (".min_connections(connections)", ".max_connections(connections)", "let connections = u32::try_from(slots + 2)?",
          "sleep(Duration::from_millis(job.hold_ms)).await", "drop(conn);")),
        (APALIS / "crates/apalis-postgres-1.2.0/src/pubsub.rs", ("PgListener::connect_with(&pool)",)),
        (next((APALIS / "cargo/registry/src").glob("*/sqlx-postgres-*/src/listener.rs")),
         ("let mut connection = pool.acquire().await?;",)),
        (ROOT / "scripts/fixtures/jobs-parity-oban/lib/jobs_parity_oban.ex",
         ("pool_size = if slots, do: slots + 2", "notifier: Oban.Notifiers.Isolated")),
        (ROOT / "scripts/fixtures/jobs-parity-oban/lib/worker.ex", ("Repo.checkout(fn ->", "Process.sleep(hold_ms)")),
        (ROOT / "target/linode-jobs-native/bundles/v5/source/proof/jobs-parity/linux-regression/oban-session.ex.in",
         ("pool_size: 1", "jobs_parity_oban:producer_observer")),
    )
    result = {}
    for path, fragments in files:
        text = path.read_text()
        require(all(fragment in text for fragment in fragments), f"source anchor changed {path}")
        result[str(path.relative_to(ROOT))] = dict(sha256=run.digest(path), anchors=fragments)
    return result


def proof(sources):
    # Application-held sockets form part of each peer runner pool. The two
    # River listeners are the only additional runner sockets here.
    roles = {
        "mesh": dict(application_pool=8, queue_and_listen=8, detached=0, producer_observer=1),
        "oban": dict(application_pool=64, queue_and_listen=4, detached=0, producer_observer=1),
        "apalis": dict(application_pool=64, queue_and_listen=4, detached=0, producer_observer=1),
        "river": dict(application_pool=64, queue_and_listen=4, detached=2, producer_observer=1),
    }
    caps = {engine: sum(counts.values()) for engine, counts in roles.items()}
    require(CAP == max(caps.values()) and all(caps[e] > 68 for e in ("river", "oban", "apalis")),
            "expected capped sensitivity does not retain stock peer pools")
    result = dict(**native_receipts(), per_engine=roles, max_simultaneous_physical=caps,
                  administrator={"before_and_after": 1, "while_timed": 0}, common_sensitivity_cap=CAP)
    if sources:
        result["protocol_source"] = source_receipts()
    return result


def check_log_parser():
    # Parser unit witness only. It cannot enter a score or native archive replay.
    stamp = "2026-11-24 01:00:00."
    start = 1795482000200000
    rows = [f"{stamp}100 UTC [{pid}] app | LOG:  connection authorized: "
            f"user=parity_subject database=river application_name={name} SSL enabled"
            for pid, name in enumerate(("river:producer_observer", "river:parity_0123456789abcdef:runner-0",
                                        "river:parity_0123456789abcdef:runner-1"), 100)]
    rows += [f"{stamp}900 UTC [{pid}] app | LOG:  disconnection: session time: 0:00:00.800" for pid in range(100, 103)]
    result = physical("\n".join(rows), "go", start, start+500000)
    require(result["peak"] == 3 and result["event_counts"] == {"connect": 3, "disconnect": 3},
            "physical parser does not account for the shared observer")
    try:
        physical("\n".join(rows[:-1]), "go", start, start+500000)
    except ValueError:
        return
    raise ValueError("unaudited retained connection accepted")


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--sources", action="store_true")
    args = cli.parse_args()
    check_log_parser()
    print(json.dumps(proof(args.sources), sort_keys=True, indent=2))
