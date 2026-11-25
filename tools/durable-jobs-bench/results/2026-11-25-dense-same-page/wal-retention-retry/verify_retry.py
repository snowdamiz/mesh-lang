"""Offline assertions against the frozen raw archives, grader, and WAL witness."""
import json
import subprocess

import controller as c
from wal_window import window



def check(cfg):
    subprocess.run(["bash", "-n", c.HERE / "launch.sh"], check=True)
    for file in c.HERE.glob("*.py"):
        compile(file.read_text(), str(file), "exec")
    original = c.original_failure()
    block = c.original.BLOCKS[0]
    failures = {}
    old = c.matrix.replay
    try:
        c.matrix.replay = c.block_replay
        selection = c.matrix.select_attempt(c.ORIGINAL_OUT, cfg, block, failures)
    finally:
        c.matrix.replay = old
    assert selection is None and list(failures.values()) == [f"warmup/default-a: {original['graded_rejection']}"]

    base = c.original.BASELINE
    prior = c.run.read_json(base / "series.json")["config"]
    prior["lanes"] = {"default-a": dict(engine="mesh", runners=2, inventory=c.run.read_json(base / "staging/hot/inventory.json"))}
    old_arm = sorted(base.glob("blocks/*/measured-hot/measured-mesh/raw.tar.gz"))[0].parent
    hot = c.frozen_score(old_arm, "measured", "default-a", prior)
    assert hot["completed"] == 1024 and hot["checkout_probes"] == 4

    from worker import redact_json
    assert redact_json(dict(terminal_token_state_ok=True, terminal_lease_state_ok=True)) == dict(terminal_lease_state_ok=True)

    proof = c.run.read_json(c.HERE / "evidence/wal-proof.json")
    state = proof["range"]
    start, end = state["start_lsn"], state["end_lsn"]
    assert proof["first_segment_retired"] and proof["stopped_cleanly"] and proof["original_exit"] == 1
    assert state["files"][0] in proof["original_stderr"]
    assert window(start, end, state) == state["files"] and len(state["files"]) == 2
    assert c.original.reduce("\n".join(proof["captured"])) == proof["counts"]
    assert {key: proof["counts"][key] for key in ("inserts", "updates", "same_page", "prefix", "hot")} == dict(
        inserts=2, updates=1, same_page=1, prefix=1, hot=0)
    unflushed = dict(state, flush_lsn=start)
    try:
        window(start, end, unflushed)
    except ValueError as error:
        assert str(error) == "invalid or unflushed WAL window"
    else:
        raise AssertionError("snapshot accepted an unflushed endpoint")

    structure = c.original.structure()
    for label, definition in cfg["lanes"].items():
        assert label in c.original.LANES and c.WALDUMP in definition["inventory"]["engines"]["mesh"]["verified_files"]
    print(json.dumps(dict(original=original, frozen_grader_rejection=failures,
                          prior_hot_raw_sha256=hot["raw_sha256"],
                          window_counts=proof["counts"], rejected_unflushed=True,
                          structural_complete_prefix=sum(row[p]["full_argument_prefix"]
                                                         for row in structure["fill50"]
                                                         for p in ("claim", "complete")),
                          rejected_decision_controls=c.original.check_gate()), indent=2))
    return 0
