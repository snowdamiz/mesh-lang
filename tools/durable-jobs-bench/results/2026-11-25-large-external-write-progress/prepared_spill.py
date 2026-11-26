"""Compare the public prepared database counters with the earlier paired V1 memory trial."""
from hashlib import sha256
from io import BytesIO
import json
from statistics import median
import subprocess
import tarfile

from render_budget import COMMIT, PILOT, ROOT, blob


V1 = ROOT / "tools/durable-jobs-bench/results/2026-11-23-ordinary-large-workmem/evidence/ordinary-large-workmem-matched"
SCORE = "ff2f7f59835cd8767309d2f8c54636647c21e6898675fb84d40f10691be25a13"
RELEASE = "89252dc00d74985d5362dfee823041f39f3079e6a7378d57d83dae314683d5b8"
V1_SCORE = "23a26322ac3e8306e1a40626b551551546ba3008d9023f9ba7850a59d126a6bf"
V1_RELEASE = "c32bbf43d30e304347f71e9a2ad0ca8e1098ea93424acf031511c8b296f3f51f"


def archive_json(archive, key):
    return json.load(archive.extractfile(f"adapter/{key}.json"))


def public_arm(archive, prepared):
    topology = archive_json(archive, "storage-topology")
    assert topology["args_storage"] == "e" and topology["work_mem"] == "4MB"
    ledger = archive_json(archive, "ledger")
    txn, = ledger["transactions"]
    jobs = ledger["jobs"]
    assert len(jobs) == 64 and txn["enqueued_ids"] == {j["job_key"]: j["id"] for j in jobs}
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid", "settled_xid")}) == 192
    assert all(j["state"] == "completed" and j["attempts"] == j["effects"] == 1 for j in jobs)
    effects = sorted(j["effect_committed_us"] - txn["committed_us"] for j in jobs)
    result = dict(effect8_us=effects[7], effect32_us=effects[31], effect64_us=effects[-1],
                  drain_us=max(j["settled_us"] for j in jobs) - txn["committed_us"])
    if "adapter/database-temp.json" in archive.getnames():
        result["database_temp"] = archive_json(archive, "database-temp")
    if prepared:
        broker, *runners = archive_json(archive, "process-identity")
        elf, = {r["executable_sha256"] for r in runners}
        assert len(runners) == 8 and elf.startswith("c8791a99")
        result.update(runner_elf=elf, broker_elf=broker["executable_sha256"])
        peaks = []
        for i, runner in enumerate(runners):
            lines = archive.extractfile(f"adapter/runner-{i}.log").read().splitlines()
            assert lines[0] == b"READY"
            report = json.loads(lines[-1])
            assert report["batch_protocol"] == "locked_prepare_pg17_v1" and report["drained"]
            assert report["finished"] == sum(j["worker_pid"] == runner["pid"] for j in jobs)
            peaks.append(report["peak_queued_context_bytes"])
        result["runner_peak_queued_bytes"] = peaks
        result["runners_over_4mib"] = sum(v > 4 * 1024**2 for v in peaks)
        statements = archive_json(archive, "queue-policy")["queue_statements"]
        for fn in ("prepare_batch_v1", "activate_batch_v1"):
            st, = [q for q in statements if f"mesh_jobs.{fn}(" in q["query"]]
            result[fn] = {k: st[k] for k in ("calls", "rows", "execution_ms")}
    return result


def public():
    release = json.loads(blob(f"{PILOT}/controller-release.json", RELEASE))
    score = json.loads(blob(f"{PILOT}/scored-before-release.json", SCORE))
    assert release["exit_status"] == 0 and score["advanced"] is False
    paths = subprocess.check_output(["git", "ls-tree", "-r", "--name-only", COMMIT, f"{PILOT}/blocks"],
                                    text=True, cwd=ROOT).splitlines()
    result = []
    for group in score["groups"]:
        for phase, lanes in group["raw_sha256"].items():
            for lane, digest in lanes.items():
                path, = [p for p in paths if f"/{group['group']:02d}-large." in p and
                         p.endswith(f"/{phase}-{lane}/{phase}-mesh/raw.tar.gz")]
                with tarfile.open(fileobj=BytesIO(blob(path, digest))) as archive:
                    row = public_arm(archive, lane in ("both", "consumer"))
                if phase == "measured":
                    assert row["drain_us"] == round(group["arms"][lane]["drain_ms"] * 1000)
                result.append(dict(group=group["group"], group_passed=group["gates"]["passed"],
                                   phase=phase, lane=lane, raw_sha256=digest, **row))
    return result


def checked_file(path, expected):
    data = path.read_bytes()
    assert sha256(data).hexdigest() == expected, path
    return data


def historical():
    release = json.loads(checked_file(V1 / "release.json", V1_RELEASE))
    summary = json.loads(checked_file(V1 / "summary.json", V1_SCORE))
    assert release["exit_status"] == 0 and not summary["issues"] and not summary["missing"]
    result = []
    for group in summary["rows"]["large"]:
        arms = {}
        for lane in ("ordinary", "ordinary-4mb-a", "ordinary-4mb-b", "oban-external", "river-external"):
            engine = {"oban-external": "oban", "river-external": "go"}.get(lane, "mesh")
            arm_dir = f"measured-{lane}/measured-{engine}"
            digest = group["raw_sha256"][arm_dir]
            raw = checked_file(V1 / "blocks" / group["attempt"] / arm_dir / "raw.tar.gz", digest)
            pg = group["postgres"]["measured"][lane]
            sc = group["scores"][lane]
            with tarfile.open(fileobj=BytesIO(raw)) as archive:
                mem = archive_json(archive, "workmem")
            assert pg["raw_sha256"] == digest and pg["mesh_claim"] == mem["io"]["mesh_claim"]
            assert pg["work_mem_mib"] == mem["expected_mb"] == (4 if "4mb" in lane else 16)
            writes = pg["application_commits" if engine == "mesh" else "committed_application_writes"]
            assert writes == 128 and pg["completed"] == 64
            arms[lane] = dict(raw_sha256=digest, submit_ms=pg["submit_to_terminal_ms"],
                              drain_ms=pg["commit_to_terminal_ms"], claim_temp=pg["mesh_claim"],
                              pg_peak_pss_kib=pg["peak_postgres_pss_kib"],
                              pinned_cpu_ms=round(64 * (sc["pinned_kernel_cpu_ms_per_job"] +
                                                        sc["pinned_user_cpu_ms_per_job"])))
        gains = {k: round(min(arms["ordinary-4mb-a"][k], arms["ordinary-4mb-b"][k]) - arms["ordinary"][k], 3)
                 for k in ("submit_ms", "drain_ms", "pinned_cpu_ms")}
        result.append(dict(group=group["group"], lanes=arms, gain_over_better_duplicate=gains))
    return result


if __name__ == "__main__":
    history = historical()
    gains = {k: median(g["gain_over_better_duplicate"][k] for g in history)
             for k in history[0]["gain_over_better_duplicate"]}
    print(json.dumps(dict(public_factorial=public(), historical_v1=history,
                          historical_median_gain_over_better_duplicate=gains), indent=2))
