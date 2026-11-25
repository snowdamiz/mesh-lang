"""Recover the completed stock rotation without changing the failed release."""
from collections import Counter
import json
from pathlib import Path
import sys
import tarfile

HERE = Path(__file__).resolve().parent
RECOVERY = HERE / "procfd-recovery"
sys.path.insert(0, str(RECOVERY))
import controller as recovery

NATIVE = RECOVERY / "evidence/native"
run = recovery.run
need = recovery.need


def replay():
    recovery.packet()
    setup, config = recovery.prior(), recovery.configuration()
    previous_path = NATIVE / "controller-release-before-dispatch.json"
    release_path = NATIVE / "controller-release.json"
    previous, release = map(run.read_json, (previous_path, release_path))
    need(previous["exit_status"] == release["exit_status"] == 1
         and previous["released_at"] < release["started"]
         and release["prior_release_sha256"] == run.digest(previous_path)
         and release["request_sha256"] == previous["request_sha256"] == run.digest(recovery.PACKET)
         and set(release["groups"]) == {1, 2}, "controller failure receipts changed")
    need(run.read_json(NATIVE / "series.json") == dict(config=config, schedule=recovery.base.BLOCKS),
         "native scope or schedule changed")

    group_one, group_two = (NATIVE / "blocks" / run.block_prefix(b) for b in recovery.base.BLOCKS)
    first = recovery.base.replay_block(group_one, recovery.base.BLOCKS[0], config, setup)
    reduction = group_one / "reconciled.json"
    need(first == run.read_json(reduction), "completed stock group changed")
    arms = first["arms"]
    sampled = arms["sampled"]["observation"]
    grants = [s for s in sampled["samples"] if (s["category"], s["window"]) == ("grant", "drain")]
    key = dict(dbid="16424", queryid=6827047345048828466, role="parity_subject",
               toplevel=False, userid="16421")
    need(len(grants) == 25 and all(s["statement_keys"] == [key] for s in grants),
         "stock grant statement key or coverage changed")
    for sample in grants:
        update, = (n for n in sample["nodes"] if n.get("Subplan Name") == "CTE granted")
        target, = (n for n in sample["nodes"] if n["subtree_path"][:len(update["subtree_path"])]
                   == update["subtree_path"] and n["subtree_path"] != update["subtree_path"]
                   and n.get("Relation Name") == "jobs")
        need(target["Node Type"] == "Index Scan" and target["Index Name"] == "jobs_pkey"
             and target["Index Cond"] == "(j_1.namespace = $1)"
             and (target["Actual Rows"], target["Actual Loops"]) == (1024, 1)
             and any(n["subtree_path"][:len(update["subtree_path"])] == update["subtree_path"]
                     and n.get("Join Filter") == "(s_1.id = j_1.id)" for n in sample["nodes"]),
             "stock UPDATE target plan changed")

    need(run.read_json(group_two / "order.json") == recovery.base.BLOCKS[1]
         and not (group_two / "off-a").exists() and not (group_two / "reconciled.json").exists(),
         "second group boundary changed")
    second_control = recovery.score_arm(group_two / "off-b/measured-mesh", config, "off-b", setup)
    failed = group_two / "sampled/measured-mesh"
    raw = failed / "raw.tar.gz"
    need(run.read_json(failed / "dispatch.json")["exit_code"] == 1,
         "second sampled dispatch exit changed")
    with tarfile.open(raw) as archive:
        status = run.file_json(archive, "status.json")
        failure = run.file_json(archive, "adapter/failure.json")
        tls = run.file_json(archive, "adapter/postgres-tls-sessions.json")
        absent = ["adapter/" + n for n in ("ledger.json", "grant-roster.json", "grant-plans.json")]
        need(status["exit_code"] == 1 and not set(absent).intersection(archive.getnames())
             and failure["type"] == "AssertionError"
             and failure["stage"][-1] == ["observe.py", 138, "capture_runners"]
             and all(s["tls"] for s in tls), "second sample no longer matches the startup stop")
    observer = recovery.rewrite_observer((HERE / "observe.py").read_text()).splitlines()
    check = "\n".join(observer[137:140]).strip()
    need(check.startswith('assert {(row["backend"], row["role"], row["tls"]) for row in standard}'),
         "failure line moved away from TLS census equality")
    counts = Counter(s["role"] for s in tls)
    need(counts == {"compiled_caller": 32, "parity_producer": 1, "parity_subject": 6},
         "stock TLS census changed")
    return dict(
        native_release=dict(exit_status=release["exit_status"], request_sha256=release["request_sha256"],
                            released_at=release["released_at"], sha256=run.digest(release_path),
                            earlier_exit_sha256=run.digest(previous_path)),
        group_one=dict(
            raw_sha256={lane: data["raw_sha256"] for lane, data in arms.items()},
            completed={lane: data["completed"] for lane, data in arms.items()},
            reduction_sha256=run.digest(reduction), overhead=first["overhead"],
            nested_grant_key=key, drain_plan_samples=len(grants), target_bitmap_samples=0),
        group_two=dict(
            off_b_raw_sha256=second_control["raw_sha256"], sampled_raw_sha256=run.digest(raw),
            sampled_exit_status=status["exit_code"], failure=failure, generated_assertion=check,
            stock_tls_census=dict(counts), later_required_tls_counts=recovery.BOTH,
            later_roster_persisted=False, final_off_a_launched=False),
    )


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
