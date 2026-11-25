"""Bound prepared TIMEOUTS calls inside the scored LARGE clock using the runner SQL bracket."""
from io import BytesIO
import json
import subprocess
import tarfile

from render_budget import COMMIT, PILOT, ROOT, blob


def count(path, digest):
    with tarfile.open(fileobj=BytesIO(blob(path, digest))) as archive:
        def read(name):
            return json.load(archive.extractfile(f"adapter/{name}.json"))

        raw, ledger, policy = read("raw"), read("ledger"), read("queue-policy")
        statements = policy["queue_statements"]

        def find(fragment):
            statement, = [row for row in statements if fragment in row["query"]]
            return statement

        timeout = find("FROM pg_catalog.pg_settings")
        turns = timeout["calls"]
        due, prepare = find("WITH clock AS MATERIALIZED"), find("FROM mesh_jobs.prepare_batch_v1")
        activate, contention = find("FROM mesh_jobs.activate_batch_v1"), find("SELECT EXISTS")
        commit, = [row for row in statements if row["query"] == "COMMIT"]
        assert all(row["calls"] == turns for row in (due, prepare, find("BEGIN ISOLATION"), commit))
        assert activate["calls"] + contention["calls"] == turns and timeout["rows"] == 2 * turns
        jobs = ledger["jobs"]
        settle = find("mesh_jobs.settle_state(")
        assert len(jobs) == len({j["settled_xid"] for j in jobs}) == settle["calls"] == settle["rows"] == 64
        assert all(j["state"] == "completed" and j["attempts"] == j["effects"] == 1 for j in jobs)
        assert prepare["rows"] == 64 and activate["rows"] == 64 + activate["calls"]
        assert raw["statement_counter_deallocations"] == 0
        assert ledger["runners"] == len(raw["wakeups"]) == 8
        assert all(w["notification_scans"] == 1 and w["subscription_scans"] == 1 for w in raw["wakeups"])

        # One outstanding claim per runner. Its six ordered statements have exactly
        # one TIMEOUTS. A slice containing k such calls covers at least 6*k-5
        # statements from that lane. Unrelated queries only widen the upper bound.
        runner_calls = raw["raw_runner_sql_calls"]
        max_timeouts = min(turns, (runner_calls - settle["calls"] + 5 * ledger["runners"]) // 6)
        return dict(raw_sha256=digest, runner_sql_bracket=runner_calls, terminal_calls=settle["calls"],
                    total_timeouts=turns, maximum_timeout_completions_in_scored_clock=max_timeouts,
                    minimum_timeout_completions_outside_scored_clock=turns - max_timeouts,
                    productive_turns=activate["calls"], total_timeout_executor_ms=round(timeout["execution_ms"], 3),
                    whole_trial_mean_times_maximum_calls_ms=round(timeout["execution_ms"] * max_timeouts / turns, 3))


def replay():
    release = json.loads(blob(f"{PILOT}/controller-release.json",
                              "89252dc00d74985d5362dfee823041f39f3079e6a7378d57d83dae314683d5b8"))
    score = json.loads(blob(f"{PILOT}/scored-before-release.json",
                            "ff2f7f59835cd8767309d2f8c54636647c21e6898675fb84d40f10691be25a13"))
    assert release["exit_status"] == 0 and score["advanced"] is False
    paths = subprocess.check_output(["git", "ls-tree", "-r", "--name-only", COMMIT, PILOT],
                                    cwd=ROOT, text=True).splitlines()
    results = []
    for group in score["groups"]:
        for phase in ("warmup", "measured"):
            for lane in ("consumer", "both"):
                suffix = f"/{phase}-{lane}/{phase}-mesh/raw.tar.gz"
                path, = [p for p in paths if f"/{group['group']:02d}-large." in p and p.endswith(suffix)]
                results.append(dict(group=group["group"], phase=phase, lane=lane,
                                    **count(path, group["raw_sha256"][phase][lane])))
    return results


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
