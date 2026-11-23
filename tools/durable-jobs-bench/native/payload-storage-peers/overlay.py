#!/usr/bin/env python3
"""Attach the storage treatment to exact stock launchers without editing them."""

import hashlib
from pathlib import Path
import runpy
import sys

import replay
import storage

STOCK_SOURCE = {
    "oban": "95766ac87a9f04dfdd253af52a04d83438d970da7d01c6cdadfea682cebb473e",
    "go": "e97ede452fb9df9c62fc9b12c914dbb2b5c92b2cd64165b7d11ac5e2dcc5d030",
    "rust": "05c94874455c7c48e0ca6e9204b412aa67d41096b6cf314c0d4a9f3a8fdaf0f7",
}
HOOKS = {
    "go": (
        ('            cluster.sql("postgres", "SELECT pg_stat_statements_reset()")',
         "            storage_install = storage.install_river(cluster)\n{}"),
        ('            ledger, _ = verify_run(raw, args.profile, binary_hash, smoke=args.smoke)',
         "{}\n            storage.finish_river(cluster, storage_install, out)"
         "\n            replay.verify_finished(out, 'go', ledger)"),
    ),
    "rust": (
        ('    parity.checked([str(binary), "migrate"], env=worker_env, log=cluster.scratch / "migrate.log")',
         '{}\n    cluster.storage_setup = storage.apply(cluster, "rust", ENGINE, "apalis")'),
        ('            run_arm(args.profile, output, binary, cluster, profile, build)',
         '{}\n            storage.finish(cluster, "rust", ENGINE, cluster.storage_setup, output)'),
    ),
}


def stock_main(engine, path):
    data = path.read_bytes()
    if hashlib.sha256(data).hexdigest() != STOCK_SOURCE[engine]:
        raise ValueError(f"{engine}: stock launcher hash changed")
    sys.path.insert(0, str(path.parent))
    if engine != "oban":
        source = data.decode()
        for anchor, inject in HOOKS[engine]:
            if source.count(anchor) != 1:
                raise ValueError(f"{engine}: missing unique overlay anchor")
            source = source.replace(anchor, inject.format(anchor))
        execute = {"__file__": str(path), "__name__": "__main__", "storage": storage, "replay": replay}
        exec(compile(source, path, "exec"), execute)
        return

    # Use subclasses of the loaded adapter, so its broker, native affinity
    # checks, permissions, log redaction and teardown stay in the stock code.
    main = runpy.run_path(str(path))["main"]
    stock = main.__globals__

    class Subject(stock["Subject"]):
        def call(self, action, env, log):
            result = super().call(action, env, log)
            if action == "migrate":
                schema = env["JOBS_PARITY_NAMESPACE"]
                self.cluster.storage_setup = storage.apply(self.cluster, "oban", self.engine, schema)
            return result

    class Cluster(stock["Cluster"]):
        def reconcile_trial(self, engine, namespace, enqueued_ids, handler_attempts, commits):
            result = super().reconcile_trial(engine, namespace, enqueued_ids, handler_attempts, commits)
            storage.finish(self, "oban", engine, self.storage_setup, self.evidence)
            return result

    stock.update(Subject=Subject, Cluster=Cluster)
    main()


def main():
    engine, path, *args = sys.argv[1:]
    if len(args) < 2 or args[-2] != "large" or "--smoke" in args:
        raise SystemExit("this overlay supports only the frozen large workload")
    if engine == "oban" and args[:-2] not in (["oban"], ["--local-smoke", "oban"]):
        raise SystemExit("the Oban overlay requires the stock Oban launcher")
    launcher = Path(path).resolve(strict=True)
    sys.argv = [str(launcher), *args]
    stock_main(engine, launcher)
    if engine != "go":
        replay.verify_finished(Path(args[-1]), engine)


if __name__ == "__main__":
    main()
