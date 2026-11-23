"""Keep the V1 executable and runnable work fixed while varying the SQL grant width."""
import argparse
import fcntl
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
TOPOLOGY = HERE.parent / "2026-11-23-ordinary-runner-topology"
sys.path.insert(0, str(TOPOLOGY))
import arm as topology
import local as baseline

AREA = ROOT / "target/ordinary-claim-width"
SOURCE = ROOT / "target/ordinary-trusted-mesh-queue/source"
BINS = ROOT / "target/ordinary-runner-topology/local-artifacts"
LAYOUT = {"eight_width8": "stock-a", "two_width8": "tuned-2x32",
          "two_width16_a": "tuned-2x32", "two_width16_b": "tuned-2x32"}
ORDERS = [
    ["two_width16_a", "two_width8", "eight_width8", "two_width16_b"],
    ["two_width16_b", "eight_width8", "two_width8", "two_width16_a"],
    ["two_width8", "two_width16_b", "two_width16_a", "eight_width8"],
    ["eight_width8", "two_width16_a", "two_width16_b", "two_width8"],
]
IDENTITY = "'mesh_jobs.claim_batch_v1(text,text,jsonb,bigint,integer,bigint)'::regprocedure"
BEFORE = "remaining := least(coalesce(slots, 0), 16);"


def prepare(cluster, database, label):
    if cluster.broker or not cluster.owned(database):
        raise RuntimeError("claim overlay needs a new fixture-owned local database before broker startup")
    source = cluster.sql(database, "SELECT pg_get_functiondef(" + IDENTITY + ")")
    if source.count(BEFORE) != 1:
        raise ValueError("unexpected claim function before workload")
    # Eight-runner stock already caps each grant at eight locally.
    change = label == "two_width8"
    expected = source.replace(BEFORE, BEFORE.replace("16", "8")) if change else source
    if change:
        cluster.sql(database, expected)
    actual = cluster.sql(database, "SELECT pg_get_functiondef(" + IDENTITY + ")")
    if actual != expected:
        raise ValueError("installed definition differs")
    return {k: hashlib.sha256(text.encode()).hexdigest()
            for k, text in (("stock_source", source), ("installed_source", actual))}


def launch(label, directory):
    original = topology.load_arm
    def with_grant_width(src):
        adapter = original(src)
        class Subject(adapter.Subject):
            def call(self, action, env, log):
                result = super().call(action, env, log)
                if action == "migrate":
                    proof = prepare(self.cluster, self.engine, label)
                    topology.write(directory / "grant-width.json", dict(label=label, **proof))
                return result
        adapter.Subject = Subject
        return adapter
    topology.load_arm = with_grant_width
    topology.run(SOURCE, LAYOUT[label], directory, True, False)


def dispatch(directory):
    pins = baseline.check_pins(SOURCE, BINS)
    directory.mkdir()
    state = directory / "state"
    state.mkdir()
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(("MESH_", "JOBS_PARITY_", "NATIVE_JOBS_", "DURABLE_JOBS_", "PG"))}
    env.update(NATIVE_JOBS_ARTIFACTS=str(BINS), NATIVE_JOBS_STATE=str(state), TMPDIR=str(state),
               PG_BIN_DIR="/opt/homebrew/opt/postgresql@17/bin", DURABLE_JOBS_PHASE="diagnostic",
               PYTHONDONTWRITEBYTECODE="1")
    with (ROOT / "target/queue-performance.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        topology.write(directory / "series.json", dict(orders=ORDERS, layout=LAYOUT, pins=pins,
                       input=topology.INPUT, screen_sha256=topology.sha(Path(__file__)),
                       postgres_sha256=topology.sha(Path(env["PG_BIN_DIR"]) / "postgres")))
        for group, order in enumerate(ORDERS, 1):
            folder = directory / f"{group:02}"
            folder.mkdir()
            for label in order:
                arm = folder / label
                start = time.monotonic()
                load = os.getloadavg()
                with arm.with_suffix(".log").open("xb") as log:
                    status = subprocess.run([sys.executable, "-B", __file__, str(arm), "--arm", label],
                                            env=env, stdout=log, stderr=subprocess.STDOUT)
                topology.write(arm.with_suffix(".status.json"), dict(exit_code=status.returncode, load_before=load,
                               load_after=os.getloadavg(), wall_s=time.monotonic()-start))
                print(group, label, status.returncode, flush=True)
                status.check_returncode()
        assert baseline.check_pins(SOURCE, BINS) == pins


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--arm", choices=LAYOUT)
    args = parser.parse_args()
    out = args.directory.resolve()
    if not out.is_relative_to(AREA) or out == AREA or out.exists():
        raise ValueError("use a new evidence directory under target/ordinary-claim-width")
    if args.arm:
        launch(args.arm, out)
    else:
        dispatch(out)
