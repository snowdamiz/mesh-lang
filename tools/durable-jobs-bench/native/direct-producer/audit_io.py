#!/usr/bin/env python3
"""Capture file-open and read-byte witnesses outside the trace-off series."""

import argparse
import inspect
import json
from pathlib import Path
import re
import tarfile

BATCH = re.compile(r"</(?P<path>[^<>]*?/batch_[0-9]{6}\.json)>")
LINE = re.compile(r"^\s*(?P<tid>[0-9]+)\s+(?P<seconds>[0-9]+)\.(?P<us>[0-9]{6})\s+(?P<call>.*)$")
RETURN = re.compile(r"=\s+(?P<value>[0-9]+)(?:<[^<>]*>)?\s*$")
UNFINISHED = " <unfinished ...>"
RESUMED = re.compile(r"<\.\.\. (\w+) resumed>(.*)")
DURATION = re.compile(r" <([0-9]+)\.([0-9]{6})>$")


def trace_worker(worker, output):
    source = worker.read_text()
    anchor = '    with (raw / "host-samples.jsonl").open'
    if source.count(anchor) != 1:
        raise ValueError("canonical worker command anchor changed")
    source = source.replace(anchor, '''    if args.phase != "diagnostic":
        raise ValueError("fixture input tracing is diagnostic only")
    cmd = ["/usr/bin/strace", "-f", "-qq", "-ttt", "-T", "-s", "0", "-y",
           "-e", "trace=open,openat,read,pread64,readv",
           "-o", str(raw / "input-syscalls.tmp"), *cmd]
''' + anchor)
    anchor = '        write_json(raw / "status.json",'
    if source.count(anchor) != 1:
        raise ValueError("canonical worker status anchor changed")
    source = source.replace(anchor, '''        trace = raw / "input-syscalls.tmp"
        if trace.is_file():
            with trace.open() as stream:
                events = input_events(stream)
            write_json(raw / "input-syscalls.json", events)
            trace.unlink()
''' + anchor)
    definitions = ["import re", f"UNFINISHED = {UNFINISHED!r}"]
    for name in ("BATCH", "LINE", "RETURN", "RESUMED", "DURATION"):
        definitions.append(f"{name} = re.compile({globals()[name].pattern!r})")
    anchor = 'if __name__ == "__main__":\n    main()'
    output.write_text(source.replace(anchor, "\n".join(definitions) + "\n\n" +
                                     inspect.getsource(input_events) + "\n\n" + anchor))


def input_events(lines):
    events = []
    pending = {}
    for line in lines:
        match = LINE.match(line)
        if not match:
            continue
        tid = int(match["tid"])
        stamp = int(match["seconds"]) * 1_000_000 + int(match["us"])
        call = match["call"]
        if call.endswith(UNFINISHED):
            pending[tid] = (stamp, call[:-len(UNFINISHED)])
            continue
        end_us = stamp
        if resumed := RESUMED.fullmatch(call):
            if tid not in pending:
                continue
            stamp, prefix = pending.pop(tid)
            call = prefix + resumed[2]
        if duration := DURATION.search(call):
            end_us = stamp + int(duration[1]) * 1_000_000 + int(duration[2])
            call = call[:duration.start()]
        path, returned = BATCH.search(call), RETURN.search(call)
        if path and returned:
            kind = call.split("(", 1)[0]
            if kind in {"open", "openat"} and "O_RDONLY" not in call:
                continue
            filename = "/" + path["path"]
            fd = int(returned["value"]) if kind in {"open", "openat"} else int(call.split("(", 1)[1].split("<", 1)[0])
            events.append(dict(path=filename, call=kind, tid=tid, fd=fd,
                               started_us=stamp, finished_us=end_us, returned=int(returned["value"])))
    return events


def reconcile(archive_path, size, artifact_sha256):
    with tarfile.open(archive_path, "r:gz") as archive:
        def load(name):
            with archive.extractfile(name) as stream:
                return json.load(stream)
        status = load("status.json")
        ledger = load("adapter/ledger.json")
        events = load("input-syscalls.json")
    from verify import reconcile as verify_ledger
    score = verify_ledger(ledger, ledger["engine"], "large",
                          dict(version=ledger["version"], artifact_sha256=artifact_sha256))
    assert status["exit_code"] == 0 and score["completed"] == 64
    receipt, = ledger["transactions"]
    submit, commit = receipt["submitted_us"], receipt["committed_us"]
    opened, = [event for event in events if event["call"] in {"open", "openat"}
               and event["path"].endswith(f"/{receipt['request_id']}.json")
               and submit <= event["started_us"] <= commit]
    reads = [event for event in events if event["call"] == "read"
             and event["path"] == opened["path"] and event["fd"] == opened["fd"]
             and opened["finished_us"] <= event["started_us"] <= event["finished_us"] <= commit]
    assert reads and sum(event["returned"] for event in reads) == size, (opened, reads)
    assert len(receipt["enqueued_ids"]) == len(ledger["jobs"]) == 64
    return dict(engine=ledger["engine"], artifact_sha256=ledger["artifact_sha256"],
                input_sha256=ledger["input_sha256"], input_bytes=size, submit_us=submit,
                commit_us=commit, opened=opened, reads=reads)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    generate = sub.add_parser("generate")
    generate.add_argument("worker", type=Path)
    generate.add_argument("output", type=Path)
    replay = sub.add_parser("reconcile")
    replay.add_argument("archive", type=Path)
    replay.add_argument("size", type=int)
    replay.add_argument("artifact_sha256")
    args = parser.parse_args()
    if args.action == "generate":
        trace_worker(args.worker, args.output)
    else:
        print(json.dumps(reconcile(args.archive, args.size, args.artifact_sha256), indent=2))


if __name__ == "__main__":
    main()
