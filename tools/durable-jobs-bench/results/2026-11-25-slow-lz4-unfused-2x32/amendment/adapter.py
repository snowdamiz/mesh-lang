"""Compose the released stock cap-71 launcher with the isolated codec probes."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from types import ModuleType

import cap71_adapter as cap71
import overlay as codec
import storage

DB = {"mesh": "mesh", "oban": "oban", "go": "river", "rust": "apalis"}


def now():
    return dict(monotonic_ns=time.monotonic_ns(), epoch_ns=time.time_ns())


def river_run(command, *, env, check, out, marks):
    """Observe both durable runner ready files before the stock idle interval."""
    marks["runner_launch_dispatch"] = now()
    ready = [out / "river/measured" / f"runner-{i}.ready" for i in range(2)]
    # Signal handling and child lifetime follow the enclosing recorder's process group.
    with subprocess.Popen(command, env=env) as proc:
        seen = {}
        affinity = os.sched_getaffinity(0)
        os.sched_setaffinity(0, {2})
        try:
            while proc.poll() is None and len(seen) != len(ready):
                for file in ready:
                    if file.name not in seen and file.is_file():
                        seen[file.name] = now()
                if len(seen) != len(ready):
                    time.sleep(0.020)
        finally:
            os.sched_setaffinity(0, affinity)
        if len(seen) == len(ready):
            marks["river_ready_observations"] = seen
            marks["runners_ready"] = max(seen.values(), key=lambda mark: mark["monotonic_ns"])
        code = proc.wait()
    if check and code:
        raise subprocess.CalledProcessError(code, command)
    return subprocess.CompletedProcess(command, code)


def load(engine, path, policy, out, marks):
    data = path.read_bytes()
    if hashlib.sha256(data).hexdigest() != cap71.LAUNCHERS[engine]:
        raise ValueError(f"changed {engine} launcher")
    source = data.decode()
    hooks = list(codec.STOCK_HOOKS.get(engine, ()))
    if engine == "go":
        hooks.append(("            result = subprocess.run(command, env=env, check=False)",
                      "            result = observed_river(command, env=env, check=False)"))
    for anchor, template in hooks:
        if source.count(anchor) != 1:
            raise ValueError(f"changed {engine} hook: {anchor.strip()}")
        source = source.replace(anchor, template.format(anchor))
    module = ModuleType("slow_equal_codec_stock")
    module.__dict__.update(__file__=str(path), local=False, policy=policy,
                           storage_setup=codec.storage_setup, storage_finish=codec.storage_finish,
                           river_install=codec.river_install, finish_river=codec.finish_river,
                           observed_river=lambda *a, **kw: river_run(*a, **kw, out=out, marks=marks))
    sys.path.insert(0, str(path.parent))
    exec(compile(source, path, "exec"), module.__dict__)
    return module


def run(engine, policy, path, args):
    if (sys.platform != "linux" or os.geteuid() == 0 or sorted(os.sched_getaffinity(0)) != [0, 1]
            or policy not in ("pglz", "lz4") or engine not in DB or args[-2] != "slow"):
        raise ValueError("requires the fenced native slow codec launch")
    out = Path(args[-1]).resolve(strict=True)
    if any(out.iterdir()):
        raise ValueError("expected an empty evidence directory")
    marks = {"adapter_start": now()}
    adapter = load(engine, path.resolve(strict=True), policy, out, marks)
    roster = None
    if engine in ("mesh", "oban"):
        cap71.mesh_or_oban(engine, adapter, out)
        stock_subject = adapter.Subject

        class Subject(stock_subject):
            def run(self, env, index, log):
                marks.setdefault("runner_launch_dispatch", now())
                return super().run(env, index, log)

            def call(self, action, env, log):
                result = super().call(action, env, log)
                if action == "migrate":
                    codec.storage_setup(self.cluster, engine, engine, env["JOBS_PARITY_NAMESPACE"])
                return result

            def capture_runners(self, processes, logs):
                marks["runners_ready"] = now()
                super().capture_runners(processes, logs)
                session_policy = self.cluster.sql(engine, "SELECT current_setting('default_toast_compression')")
                if session_policy != policy:
                    raise ValueError("observer saw the wrong producer policy")
                self.cluster.payload_setup["producer_session_policy"] = session_policy

        adapter.Subject = Subject
    else:
        roster = cap71.river_or_apalis(engine, adapter, out)
        if engine == "rust":
            launch = adapter.parity.RunnerProcess
            wait = adapter.parity.wait_for

            def tracked_launch(*args, **kwargs):
                marks.setdefault("runner_launch_dispatch", now())
                return launch(*args, **kwargs)

            def tracked_wait(predicate, deadline, description, *args, **kwargs):
                result = wait(predicate, deadline, description, *args, **kwargs)
                if description == "Apalis LISTEN readiness":
                    marks["runners_ready"] = now()
                return result

            adapter.parity.RunnerProcess = tracked_launch
            adapter.parity.wait_for = tracked_wait
    stock_cluster = adapter.Cluster
    identities = []

    class Cluster(stock_cluster):
        def configure_tls(self):
            super().configure_tls()
            with (self.data / "postgresql.conf").open("a") as stream:
                stream.write(f"\ndefault_toast_compression = '{policy}'\n")

        def start(self):
            marks["postgres_start"] = now()
            super().start()
            marks["postgres_ready"] = now()
            exe = self.bin.resolve() / "postgres"
            pid = int((self.data / "postmaster.pid").read_text().splitlines()[0])
            if Path(f"/proc/{pid}/exe").resolve(strict=True) != exe:
                raise ValueError("actual postmaster differs from selected PostgreSQL")
            keys = ("server_version_num", "default_toast_compression", "ssl", "fsync", "synchronous_commit",
                    "full_page_writes", "track_commit_timestamp", "max_connections", "reserved_connections",
                    "superuser_reserved_connections", "shared_preload_libraries")
            settings = {key: self.sql("postgres", "SHOW " + key) for key in keys}
            if settings["default_toast_compression"] != policy:
                raise ValueError("requested policy did not take effect")
            identities.append(dict(path=str(exe), sha256=storage.sha(exe.read_bytes()), settings=settings))

        def audit_tls(self, name):
            super().audit_tls(name)
            marks["verified_tls_roster"] = now()

        def reconcile_trial(self, name, namespace, enqueued_ids, handler_attempts, commits):
            ledger = super().reconcile_trial(name, namespace, enqueued_ids, handler_attempts, commits)
            codec.storage_finish(self, engine, DB[engine], self.evidence, namespace)
            return ledger

    adapter.Cluster = Cluster
    sys.argv = [str(path), *args]
    try:
        adapter.main()
        if roster is not None:
            if set(roster) != {"runner-0", "runner-1"} or len({row["pid"] for row in roster.values()}) != 2:
                raise ValueError("wrong Apalis runner roster")
            cap71.write(out / "runner-topology.json", roster)
        if len(identities) != 1:
            raise ValueError("wrong number of private PostgreSQL instances")
        receipt = json.loads((out / "payload-storage.json").read_text())
        ledger = json.loads((out / "ledger.json").read_text())
        codec.verify_codec(storage, engine, receipt["persisted"], ledger["jobs"], policy)
        cap71.write(out / "codec-postgres.json", identities[0])
    finally:
        marks["adapter_end"] = now()
        cap71.write(out / "startup-clock.json", marks)


if __name__ == "__main__":
    engine, policy, path, *args = sys.argv[1:]
    run(engine, policy, Path(path), args)
