"""Replay the released diagnostics and grant only the two missing timing groups."""

import io
import json
from pathlib import Path
import re
import runpy
import shlex
import sys
import tarfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import controller as original

from plans import HEADER, records
from roster8 import bind
from score import pilot_gate, workload

ID = original.ID + "-continuation"
INITIAL_OUT = original.OUT
OUT = HERE / "evidence/native"
RELEASE = INITIAL_OUT / "controller/controller-release.json"
PACKET = HERE / "controller-packet.json"
QUALIFICATION = HERE / "qualification.json"
GROUPS = [1, 2]
TRIAL = original.SCHEDULE[0]
TIMING = original.SCHEDULE[1:]
CLONE = "dense_grant_probe"
ROLE = "parity_subject"
LOCK = "canceling statement due to lock timeout"
FENCE = "mesh jobs namespace admission is paused or changing"
WRAPPER = """SELECT coalesce(json_agg(row_to_json(r) ORDER BY r.ordinality),'[]')
       FROM mesh_jobs.claim_batch_set_v1('dense','default','[{"worker":"parity","version":1}]'::jsonb,60000,16,0) WITH ORDINALITY r;"""
QUEUE = 'SQL statement "SELECT * FROM mesh_jobs.queues WHERE namespace = ns AND name = queue_name FOR NO KEY UPDATE"'
CONTEXT = "PL/pgSQL function mesh_jobs.claim_batch_set_v1(text,text,jsonb,bigint,integer,bigint) line 20 at SQL statement"


def normalized(lines):
    return " ".join(" ".join(lines).split())


def report_errors(archive, log):
    """Account for each PostgreSQL error in the original diagnostic export."""
    frames = original.member(archive, "grant-frames")
    closed = original.member(archive, "grant-closed")
    pins, ports = bind(original.member(archive, "grant-roster"), frames)
    by_port = {port: int(runner) for runner, values in ports.items() for port in values}
    transcript = archive.extractfile("adapter/grant-probe.log").read().decode()
    expected = re.findall(r"(?ms)^--- parity_subject, exit 3 ---\n(.*?)(?=^---|\Z)", transcript)
    assert len(expected) == 3
    for section in expected[:2]:
        assert normalized(WRAPPER.splitlines()) in normalized(section.splitlines())
        assert f"ERROR:  55P03: {LOCK}" in section
        assert QUEUE in normalized(section.splitlines()) and CONTEXT in section
    assert f"ERROR:  55000: {FENCE}" in expected[2]
    assert "mesh_jobs.fence_claim_v1() line 7 at RAISE" in expected[2]

    events = list(records(log.replace("\n[redacted]\n", "\n\t[redacted]\n")))
    ready, = [row[0] for row in events if row[5][0] == "LOG:  database system is ready to accept connections"]
    first_live = min(stamp for _, _, stamp in pins.values())
    assert ready < first_live < frames[0]["begin_us"] < closed["us_epoch"]
    proof = []
    for index, (stamp, pid, line, role, database, lines) in enumerate(events):
        if not re.match(r"^(ERROR|FATAL|PANIC):", lines[0]):
            continue
        assert len(lines) == 1 and role == ROLE and lines[0].startswith("ERROR:"), lines[0]
        message = lines[0].split(":", 1)[1].strip()
        # PostgreSQL emits context and statement as consecutive records on this
        # backend. Other backends may interleave records in the global log.
        follow = [row for row in events[index+1:index+12] if row[1] == pid][:2]
        assert [row[:5] for row in follow] == [(stamp, pid, line+offset, role, database) for offset in (1, 2)]
        context, statement = [normalized(row[5]) for row in follow]
        assert context.startswith("CONTEXT: ") and statement.startswith("STATEMENT: ")
        receipt = dict(pid=pid, db=database, at_us=stamp, log_line=line,
                       context=context, sql=statement.removeprefix("STATEMENT: "))
        if database == CLONE:
            assert ready <= stamp < first_live and pid not in pins and line == 1
            if message == LOCK:
                assert re.match(r'^CONTEXT: while locking tuple \(\d+,\d+\) in relation "queues" ', context)
                assert QUEUE in context and context.endswith(CONTEXT)
                assert receipt["sql"] == normalized(WRAPPER.splitlines())
                receipt["sqlstate"] = "55P03"
            else:
                assert message == FENCE and context == (
                    "CONTEXT: PL/pgSQL function mesh_jobs.fence_claim_v1() line 7 at RAISE")
                assert receipt["sql"] == ("UPDATE mesh_jobs.jobs SET state='executing', attempt=1, "
                                          "[redacted] AND id=md5('3')::uuid;")
                receipt["sqlstate"] = "55000"
        else:
            assert database == "mesh" and message == LOCK and pid in pins
            role_name, port, started = pins[pid]
            assert role_name == ROLE and started <= stamp <= closed["us_epoch"]
            assert QUEUE in context and context.endswith(CONTEXT)
            assert receipt["sql"] == (
                "SELECT status, id, token, context, timeout_ms, context_bytes, "
                "CASE WHEN $6::boolean AND status = 'claimed' THEN mesh_jobs.now_ms()::text END AS observed_at_ms, "
                "ordinality::text FROM mesh_jobs.claim_batch_set_v1($1, $2, $3::jsonb, "
                "$4::bigint, $5::integer, NULL) WITH ORDINALITY")
            receipt.update(runner_pid=by_port[port], backend_start_us=started)
        proof.append(receipt)
    assert len(proof) == len(re.findall(r"(?m)^.*\| (?:ERROR|FATAL|PANIC):", log))
    clone = [row for row in proof if row["db"] == CLONE]
    assert [row["sqlstate"] for row in clone] == ["55P03", "55P03", "55000"]
    assert len({row["pid"] for row in clone}) == 3
    measured = [row for row in proof if row["db"] == "mesh"]
    counts = {runner: 0 for runner in by_port.values()}
    for row in measured:
        counts[row["runner_pid"]] += 1
    for index, runner in enumerate(ports):
        member = archive.extractfile(f"adapter/runner-{index}.log")
        report = json.loads(member.read().splitlines()[-1])
        assert report["batch_contention_hints"] == counts[int(runner)]
    return dict(cluster_ready_us=ready, first_workload_session_us=first_live,
                first_observer_us=frames[0]["begin_us"], closed_us=closed["us_epoch"], errors=proof)


class MeasuredDatabaseLog:
    def __init__(self, archive, log):
        self.archive = archive
        self.log = log

    def extractfile(self, name):
        if name != "adapter/postgres.log":
            return self.archive.extractfile(name)
        lines = []
        clone = False
        for line in self.log.splitlines(keepends=True):
            if match := HEADER.match(line):
                clone = match[6] == CLONE
            elif not line.startswith("\t") and line.strip() != "[redacted]":
                clone = False
            if not clone:
                lines.append(line)
        return io.BytesIO("".join(lines).encode())


def scoped_workload(archive):
    log = archive.extractfile("adapter/postgres.log").read().decode()
    report_errors(archive, log)
    return workload(MeasuredDatabaseLog(archive, log))


def recover(cfg):
    release = original.run.read_json(RELEASE)
    assert release["id"] == original.ID and release["groups"] == [0, 1, 2]
    assert release["attempts"] == [1] and release["exit_status"] == 1
    assert "score.py" in next((INITIAL_OUT/"controller").glob("timing-*.log")).read_text()
    block, = (INITIAL_OUT/"blocks").glob("00-noop.*")
    assert not list((INITIAL_OUT/"blocks").glob("0[12]-noop.*"))
    saved = original.workload
    original.workload = scoped_workload
    try:
        result = original.score_block(block, TRIAL, cfg)
    finally:
        original.workload = saved
    assert result["plan_gate"]["passed"]
    for lane, arm in result["arms"].items():
        folder = block/f"measured-{lane}/measured-mesh"
        assert original.run.read_json(folder/"dispatch.json")["exit_code"] == 0
        with tarfile.open(folder/"raw.tar.gz") as archive:
            log = archive.extractfile("adapter/postgres.log").read().decode()
            arm["error_attribution"] = report_errors(archive, log)
    return dict(release=release, block=str(block.relative_to(original.ROOT)), **result)


def command(action):
    return shlex.join(["python3", "-B", str((HERE/"driver.py").relative_to(original.ROOT)), action])


def packet():
    block = original.ROOT / original.run.read_json(QUALIFICATION)["block"]
    files = (HERE/"driver.py", HERE/"test_recover.py", HERE/"README.md", QUALIFICATION, original.PACKET, RELEASE,
             *sorted((INITIAL_OUT/"controller").glob("timing-*.log")), block/"order.json",
             *sorted(block.rglob("raw.tar.gz")), *sorted(block.rglob("dispatch.json")), *sorted(block.rglob("request.json")))
    return dict(id=ID, controller="native-controller-complete-post-wal-queue", no_build=True,
                after_existing_bookings=True, ready=True, candidate_sha256=original.KEY_SHA,
                input_sha256=original.INPUT, evidence_destination=str(OUT), controller_receipt_dir=str(OUT/"controller"),
                offline_check=command("check"), timing_command=command("timing"), replay_command=command("replay"),
                schedule=TIMING, timing_window_minutes=12, reused_diagnostic_release_sha256=original.run.digest(RELEASE),
                local_sha256={str(p.relative_to(original.ROOT)): original.run.digest(p) for p in files})


def bind_continuation():
    original.ID = ID
    original.PACKET = PACKET
    original.OUT = OUT
    original.DIAGNOSTIC = ()
    original.SCHEDULE = TIMING
    return original.config()


def release_path():
    return original.QUEUE/f"{ID}-groups-1-2.released.json"


def intake(seal):
    assert seal == original.run.digest(PACKET)
    assert (original.QUEUE/f"{ID}.json").read_bytes() == PACKET.read_bytes()
    release = release_path()
    if release.exists():
        return original.run.read_json(release)["exit_status"]
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) ", (original.QUEUE/"STATUS.md").read_text(), re.M)
    assert events[-1] == "RELEASED"
    request = original.run.read_json(PACKET)
    request.update(timing_grant_id=ID, command_attempts=1)
    return runpy.run_path(str(original.QUEUE.parent/"completion-writer-r2.py"))["measure"](
        request, "timing_command", GROUPS)


def main(args):
    initial = original.config()
    original.stage_check(initial)
    assert original.run.read_json(original.PACKET) == original.request(initial)
    qualification = recover(initial)
    if args == ["recover"]:
        print(json.dumps(qualification, indent=2, sort_keys=True))
        return 0
    assert qualification == original.run.read_json(QUALIFICATION)
    if args == ["seal"]:
        assert not PACKET.exists()
        original.run.write_json(PACKET, packet())
        return 0
    assert original.run.read_json(PACKET) == packet()
    cfg = bind_continuation()
    if args == ["check"]:
        print(f"{ID} verified {original.run.digest(PACKET)}")
        return 0
    if args == ["replay"]:
        result = original.replay(cfg)
        result["groups"][0] = qualification
        result["pilot_gate"] = pilot_gate(result["groups"])
        path = release_path()
        result["release"] = original.run.read_json(path) if path.exists() else None
        result["fair_8x8_peers_eligible"] = bool(result["release"] and result["release"]["exit_status"] == 0
            and result["pilot_gate"]["passed"] and not result["failures"] and not result["additional_complete_attempts"])
        print(json.dumps(result, indent=2, sort_keys=True))
        return 0
    if args == ["timing"]:
        return original.dispatch(cfg)
    if len(args) == 2 and args[0] == "intake":
        return intake(args[1])
    raise SystemExit("usage: driver.py recover|seal|check|replay|timing|intake SHA")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
