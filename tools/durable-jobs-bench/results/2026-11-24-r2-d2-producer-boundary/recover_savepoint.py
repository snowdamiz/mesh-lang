"""Replay the released SETUP with PG17's normalized utility statement spelling."""
import argparse
import json
import shlex
import sys
from pathlib import Path

import controller as c
import score

ATTEMPT = "r2-d2-producer-boundary-c473e0c4bb1d"
RAW = "f7e6d1fc5c64f4b8dd17cfb5db4e39f72cb1d7e12165b53ac1ff0baecee162aa"
FOLDER = c.EVIDENCE / "setup" / ATTEMPT
OUTPUT = c.EVIDENCE / "native-timing" / ATTEMPT
ORIGINAL = score.sql_witness


def normalized(stats, mode):
    statements = []
    for row in stats["statements"]:
        if row["toplevel"] and row["query"].strip() == "RELEASE SAVEPOINT $1":
            row = dict(row, query="RELEASE SAVEPOINT mesh_jobs_enqueue")
        statements.append(row)
    return ORIGINAL(dict(stats, statements=statements), mode)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["verify", "run", "timing", "report"])
    args = parser.parse_args()
    packet, known = c.inputs()
    assert c.digest(FOLDER / "raw.tar.gz") == RAW
    score.sql_witness = normalized
    image, proof = c.verify_setup(FOLDER, known)
    c.matrix.plan, c.matrix.replay = c.plan, c.recheck_block
    if args.action == "verify":
        print(json.dumps(dict(raw_sha256=RAW, image=image, proof=proof), indent=2))
        return
    if args.action == "run":
        import runpy
        ops = runpy.run_path(str(c.QUEUE.parent / "completion-writer-r2.py"))
        release = c.run.read_json(c.QUEUE / (c.ID + ".released.json"))
        assert release["exit_status"] == 1 and release["released_at"]
        stage = dict(packet, evidence_destination=OUTPUT, controller_receipt_dir=FOLDER.parent.parent / "recovery",
                     command_attempts=1, dispatch=shlex.join([sys.executable, "-B", str(Path(__file__).resolve()), "timing"]))
        if ops["measure"](stage, "dispatch", [1, 2, 3, 4, 5]):
            raise SystemExit("Timing stopped; inspect controller and raw receipts")
    if args.action == "timing":
        c.grant("TIMING")
        config = c.configure(known, ATTEMPT, image)
        OUTPUT.parent.mkdir(parents=True, exist_ok=True)
        for groups in ([1, 2], [3, 4, 5]):
            c.grant("TIMING")
            rc = c.matrix.dispatch(OUTPUT, config, groups)
            if rc:
                raise SystemExit(rc)
            assert set(groups) <= {g["group"] for g in c.replay(OUTPUT)["groups"]}
        return
    summary = c.replay(OUTPUT)
    assert summary["complete"]
    report = OUTPUT / "summary.json"
    try:
        c.run.write_json(report, summary)
    except FileExistsError:
        assert c.run.read_json(report) == summary


if __name__ == "__main__":
    main()
