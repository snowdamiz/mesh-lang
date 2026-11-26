"""Compare pre-submit host CPU with the runner CPU shortfall in the Direct4 pilot."""
import json
import subprocess

from before_submit_cpu import measure


SOURCE = "2d96798df87180edf0ea4744b4a16116b1d301f5"
EVIDENCE = "tools/durable-jobs-bench/results/2026-11-25-dense-fixed8-pool-supply/evidence/native"


def blob(path):
    return subprocess.check_output(["git", "show", f"{SOURCE}:{EVIDENCE}/{path}"])


def replay():
    release = json.loads(blob("controller/pilot-release.json"))
    assert release["exit_status"] == 0 and release["groups"] == [1, 2]
    assert release["request_sha256"] == "5cb27783ee9d783ac64377493d2887ae0060f2fc3941c1bf3bed4a452757570f"
    entries = blob("SHA256SUMS.pilot").decode().splitlines()
    hashes = {path: digest for digest, path in (line.split(maxsplit=1) for line in entries)}
    results = []
    for group in release["groups"]:
        block = f"blocks/{group:02}-noop"
        reconciled = json.loads(blob(f"{block}/reconciled.json"))
        for phase in ("warmup", "measured"):
            rows = reconciled[phase]
            assert set(rows) == {"bridge", "service2-a", "service2-b", "service4", "direct2", "direct4"}
            arms = {}
            for lane, row in rows.items():
                path = f"{block}/{phase}-{lane}/{phase}-mesh/raw.tar.gz"
                assert hashes[path] == row["raw_sha256"] and row["app_tls"] == 32
                arms[lane] = measure(blob(path), hashes[path], river=False)
            busy_difference_pp = arms["direct4"]["busy_percent"] - arms["direct2"]["busy_percent"]
            # Nominal three-second idle period, two host CPUs, constant observed rate.
            host_difference_ms = 3000 * 2 * busy_difference_pp / 100
            cpu_ceiling = max(rows[lane]["runner_cpu_ms"] for lane in ("service2-a", "service2-b"))
            shortfall_ms = rows["direct4"]["runner_cpu_ms"] - cpu_ceiling
            results.append(dict(group=group, phase=phase, arms=arms,
                                runner_shortfall_ms=shortfall_ms,
                                projected_host_difference_ms=host_difference_ms,
                                required_share=(shortfall_ms / host_difference_ms
                                                if host_difference_ms > 0 else None)))
    return results


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2))
