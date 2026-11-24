"""Untimed checks of sealed anchors, producer stage accounting and reject paths."""
import ast
from hashlib import sha256
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
from textwrap import dedent

from adapter import trial_source
from materialize import runtime
from score import PYTHON_EVENTS, RUST_EVENTS, segments, sql_witness

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("stage_budget_controller", HERE / "controller.py")
controller = importlib.util.module_from_spec(spec)
spec.loader.exec_module(controller)


def check_anchors():
    pinned = controller.required_configuration()
    package = controller.payload(controller.native_pins(pinned))
    expected = controller.run.read_json(HERE / "evidence/offline/runtime-pins.json")
    with tarfile.open(fileobj=io.BytesIO(package), mode="r:gz") as raw:
        def content(name):
            return raw.extractfile(name).read()
        trial = content("tree-short/proof/jobs-parity/linux-regression/trial.py").decode()
        cls, = [node for node in ast.parse(trial).body if getattr(node, "name", "") == "Subject"]
        fn, = [node for node in cls.body if getattr(node, "name", "") == "warm_producer"]
        sliced = trial.splitlines(keepends=True)[fn.decorator_list[0].lineno - 1:fn.end_lineno]
        program = trial_source(dedent("".join(sliced)))
        compile(program, "sealed-observed-producer", "exec")
        compile(content("replay/split_arm.py"), "sealed-split", "exec")
        # The diagnostic source must preserve the writer and SQL verbatim.
        rel = "compiler/mesh-rt/src/jobs/"
        for file, expected_digest in {
            "store.rs": "c41b0c8e32b9861b00f46d0cbc81be5b462dbb6294da99b46f67a2eef7e7913f",
            "enqueue_schema.sql": "a09615a87facc9290b8cd9f9f1910228f85e91b8863f08c417c20c8d6abb2f73",
        }.items():
            assert sha256(content(f"tree-short/{rel}{file}")).hexdigest() == expected_digest
        with tempfile.TemporaryDirectory(dir=HERE / "evidence/offline") as temporary:
            source = Path(temporary)
            for file in expected:
                if expected[file][0] is not None:
                    dest = source / file
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    dest.write_bytes(content(f"tree-short/compiler/mesh-rt/src/{file}"))
            assert runtime(source, content("probe.rs")) == expected
    return sha256(package).hexdigest()


def check_arithmetic():
    wall_offset = 1_790_000_000_000_000_000
    rust = dict(zip(RUST_EVENTS, range(100_000, 100_000 + 10_000 * len(RUST_EVENTS), 10_000), strict=True))
    commit = rust["commit_send"] + 5000
    assert rust["commit_send"] < commit < rust["commit_sent"] < rust["commit_ready"]
    points = [10_000, 20_000, 30_000, 40_000, 110_000, 450_000, 460_000, 470_000]
    log = "\n".join(f"r2_producer_stage {key} {value}" for key, value in rust.items())
    caller = dict(before=[999, wall_offset + 1000, 1001], after=[471_999, wall_offset + 472_000, 472_001],
                  events=dict(zip(PYTHON_EVENTS, points, strict=True)))
    ledger = dict(submit_us=(wall_offset + 5_000) // 1000, commit_us=(wall_offset + commit) // 1000,
                  ack_us=(wall_offset + 475_000) // 1000)
    result = segments(log, caller, ledger)
    assert abs(sum(result["stages_ms"].values()) - 0.470) < 1e-9
    assert result["stages_ms"]["commit_send_start_to_record"] == 0.005
    assert result["stages_ms"]["commit_record_to_ready"] == 0.015
    assert result["pipe_write_ms"] > result["stages_ms"]["broker_pipe_handoff"]
    for broken in (log + "\nr2_producer_stage commit_ready 1000", log.replace("bulk_first_row", "unknown")):
        try:
            segments(broken, caller, ledger)
        except AssertionError:
            continue
        raise AssertionError("invalid wire trace accepted")


def check_sql_accounting():
    def statement(sql, elapsed, rows, *, top=True):
        return dict(query=sql, total_exec_time=elapsed, rows=rows, calls=1, toplevel=top)
    statements = [
        statement("SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1($1, $2)", 12, 128),
        statement("RELEASE SAVEPOINT mesh_jobs_enqueue", 0.1, 0),
        statement("COMMIT", 0.3, 0),
        statement("WITH inserted AS (MERGE INTO mesh_jobs.jobs...) INSERT INTO mesh_jobs.receipts...", 8, 128, top=False),
    ]
    functions = [
        dict(funcname="enqueue_batch_v1", calls=1, total_time=11, self_time=8),
        dict(funcname="enqueue_batch_layout_v1", calls=1, total_time=2, self_time=2),
        dict(funcname="publish_wake_v1", calls=128, total_time=1, self_time=1),
    ]
    result = sql_witness(dict(mode="nested", statements=statements, functions=functions,
                              role_settings=[dict(setconfig=["track_functions=pl", "pg_stat_statements.track=all"])]),
                         "nested")
    assert result["bulk_top_execution_ms"] == 12
    assert result["write_cte_inclusive_ms"] == 8
    assert result["layout_inclusive_ms"] == 2
    assert result["wake_trigger"]["total_time"] == 1


if __name__ == "__main__":
    check_arithmetic()
    check_sql_accounting()
    print(json.dumps(dict(status="PASS", checks=["frozen runtime and Popen anchors", "disjoint stage sum",
                                               "commit before send return", "invalid trace rejection",
                                               "inclusive SQL accounting"],
                          relative_clock="synthetic", setup_payload_sha256=check_anchors()), indent=2))
