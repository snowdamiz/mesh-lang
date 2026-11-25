"""Price the most CPU an isolated SQL executor optimization could remove."""
import json
import tarfile

from replay import HERE, NATIVE, digest, replay


def statement(rows, prefix, count):
    row, = [row for row in rows if row["query"].startswith(prefix)]
    assert row["calls"] == count and row["rows"] == count
    assert row["execution_ms"] > 0
    return row["execution_ms"]


def evaluate():
    budget = replay()
    assert budget == json.loads((HERE / "bounds.json").read_text())
    result = {}
    for group, lanes in budget.items():
        mesh = lanes["hot"]
        archive, = (NATIVE / "blocks").glob(f"{int(group):02}-*/measured-hot/measured-mesh/raw.tar.gz")
        assert digest(archive) == mesh["raw_sha256"]
        with tarfile.open(archive) as raw:
            app = json.load(raw.extractfile("adapter/trusted-application-statements.json"))
            policy = json.load(raw.extractfile("adapter/queue-policy.json"))
            ledger = json.load(raw.extractfile("adapter/ledger.json"))
            counters = json.load(raw.extractfile("adapter/raw.json"))
        assert counters["statement_counter_deallocations"] == 0
        assert policy["terminal_xids"] == 1024 and policy["completion_cohort_queries"] == 0
        assert len(ledger["jobs"]) == 1024
        attempts = statement(app, "INSERT INTO public.mesh_jobs_parity_attempts", 1024)
        effects = statement(app, "INSERT INTO public.mesh_jobs_parity_effects", 1024)
        settles = statement(policy["queue_statements"], "SELECT coalesce(mesh_jobs.settle_state(", 1024)
        runner_cpu = 1000 * counters["runner_cpu_seconds_including_startup_and_idle"]
        assert abs(runner_cpu - 1000 * ledger["runner_cpu_seconds"]) < 0.001
        target = (mesh["submit_busy_floor_cpu_us"] - 2 * lanes["river-2x32"]["submit_us"] / 1.2) / 1000
        result[group] = dict(required_cpu_ms=target, runner_lifetime_cpu_ms=runner_cpu,
                             attempt_insert_exec_ms=attempts, effect_insert_exec_ms=effects,
                             individual_settle_exec_ms=settles,
                             required_pct_runner_lifetime=100 * target / runner_cpu,
                             required_pct_settle_elapsed=100 * target / settles,
                             residual_after_both_inserts_cpu_ms=target - attempts - effects,
                             residual_after_inserts_and_settle_cpu_ms=target - attempts - effects - settles)
    return result


if __name__ == "__main__":
    print(json.dumps(evaluate(), indent=2, sort_keys=True))
