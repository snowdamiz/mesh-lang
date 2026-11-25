"""Replay the dense grouped writer pilot or dispatch under the sole controller's grant."""

import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import sys
import tarfile

from package import BASE, HERE, REPLAY, ROOT, sha

sys.path.insert(0, str(HERE.parent / "2026-11-24-dense-product-d2-transfer"))
import transfer as public

matrix, run, require = public.matrix, public.run, public.require
ID = HERE.name
REQUEST = HERE / "request.json"
POLICIES = {"stock-a": "", "stock-b": "", "zero": "zero", "fixed8": "fixed8ms", "hot": "refill8ms"}
PHASES = ["warmup", "measured"]
FIRST = ["stock-a", "zero", "hot", "fixed8", "stock-b"]
SCHEDULE = [dict(group=i, order=list(order), profile="noop", phases=PHASES)
            for i, order in ((1, FIRST), (2, reversed(FIRST)))]
PEER_FIRST = ["stock-a", "river-2x32", "hot", "oban-2x32", "apalis-2x32"]
PEER_SCHEDULE = [dict(group=i, order=list(order), profile="noop", phases=PHASES)
                 for i, order in ((1, PEER_FIRST), (2, reversed(PEER_FIRST)))]
PROOFS = ("sql-cut", "held-terminal", "lost-terminal", "lost-bulk", "cancellation",
          "held-teardown", "forced-teardown", "stop-partial")
public.PLAN = REQUEST
stage_product = public.stages


def packet():
    request = run.read_json(REQUEST)
    require(request["source_sha"] == BASE and request["input_sha256"] == public.INPUT
            and request["schedule"] == SCHEDULE and request["peer_schedule"] == PEER_SCHEDULE
            and request["modes"] == {name: "product-d2" for name in POLICIES},
            "packet drift")
    for path, digest in request["local_sha256"].items():
        require(sha(ROOT / path) == digest, f"changed local input {path}")
    return request


def setup(archive, release, raw):
    expected = packet()
    source_sha = sha(archive)
    require(source_sha == expected["source_archive_sha256"], "source archive changed")
    receipt = run.read_json(release)
    require(receipt["id"] == ID + "-setup" and receipt["exit_status"] == 0
            and receipt.get("raw_sha256", sha(raw)) == sha(raw), "missing successful SETUP release")
    with tarfile.open(raw) as setup_raw, tarfile.open(archive) as source:
        get = public.archive_files(setup_raw)
        src = public.archive_files(source, "source/")
        manifest = get("mesh.json")
        build = json.loads(manifest)
        witness = json.loads(get("witness.json"))
        for item in (build, witness):
            require((item["system"], item["architecture"], item["version"], item["source_sha256"],
                     item["compiler_sha256"], item["execution_contract"]) ==
                    ("Linux", "x86_64", BASE, source_sha, public.COMPILER_SHA,
                     "ordinary-trusted-shared-process-v2"), "unexpected setup build")
        require(build["runtime_sha256"] == witness["runtime_sha256"]
                and build["files"]["mesh"] != witness["files"]["mesh"], "witness runtime mismatch")
        for name in ("main.mpl", "writes.mpl", "worker_identity.mpl", "worker_identity.c"):
            data = src(str(REPLAY.parent / "fixture" / name))
            require(hashlib.sha256(data).hexdigest() == build["files"][name], f"fixture changed {name}")
        toolchain = get("toolchain.txt").decode()
        require(all(item in toolchain for item in ("rustc 1.99.0", "21.1.8", "17.11",
                    "90ec1c24-08c8-4bc1-aa28-79b6db1bd13a", "6.8.0-149-generic", "8:0")),
                "native SETUP host or toolchain changed")
        elf = get("elf.txt").decode()
        require("ELF64" in elf and "X86-64" in elf, "native ELF header missing")
        data = {name: get(f"semantic-root/semantic/{name}/result.json") for name in PROOFS}
        scores = {name: json.loads(value) for name, value in data.items()}
        validate_proofs(scores, witness["files"]["mesh"])
    return dict(build=build, manifest_sha256=hashlib.sha256(manifest).hexdigest(),
                source_sha256=source_sha, setup_raw_sha256=sha(raw), release_sha256=sha(release),
                proofs={name: hashlib.sha256(data[name]).hexdigest() for name in PROOFS})


def validate_proofs(scores, witness):
    cut = scores["sql-cut"]
    require(cut["rolled_back"] and cut["wrong_token_untouched"] and cut["expired_untouched"],
            "public SQL fencing failed")
    for name, drop in (("held-terminal", False), ("lost-terminal", True)):
        row = scores[name]
        require(row["wire"]["idle"] and row["wire"]["receipt_dropped"] == drop
                and row["capacity"] == 4 and len(row["started_while_held"]) == 3
                and row["renewal_while_held"] > row["renewal_before"]
                and row["independent_app_xids"] == 10, "held receipt qualification failed")
    bulk = scores["lost-bulk"]
    require(bulk["wire"]["idle"] and bulk["wire"]["receipt_dropped"]
            and bulk["wire"]["rows"] == 2 and bulk["bulk_submissions"] == 1
            and bulk["token_stale_confirmations"] == 2, "lost cohort receipt qualification failed")
    result = scores["cancellation"]
    require(result["cancel"]["state"] == "cancelled" and result["retry"]["attempt"] == 2,
            "cancellation or retry changed")
    for name, expected in (("held-teardown", "completed"), ("forced-teardown", "executing")):
        row = scores[name]
        require(row["runner_sha256"] == witness and row["held"]["state"] == "executing"
                and row["done"]["state"] == expected and row["held"]["effect"] == row["done"]["effect"],
                "teardown qualification failed")
    require(scores["stop-partial"]["drained"]["state"] == "completed", "partial stop failed")


def stages(config, label, remote):
    files, inventory = stage_product(config, label, remote)
    if label in POLICIES:
        argv = inventory["engines"]["mesh"]["argv"]
        argv.insert(1, f"JOBS_GROUPED_WRITER={POLICIES[label]}")
        run.check_inventory(inventory)
    return files, inventory


def occupancy(jobs):
    edges = defaultdict(Counter)
    xids = defaultdict(set)
    for job in jobs:
        pid = job["worker_pid"]
        edges[pid][job["claimed_us"]] += 1
        edges[pid][job["settled_us"]] -= 1
        xids[job["settled_xid"]].add(pid)
    peaks = {}
    for pid, timeline in {**edges, "total": sum_edges(edges.values())}.items():
        occupied = peak = 0
        for stamp in sorted(timeline):
            occupied += timeline[stamp]
            require(occupied >= 0, "negative database occupancy")
            peak = max(peak, occupied)
        require(occupied == 0 and peak <= (64 if pid == "total" else 32), "database slot cap")
        peaks[str(pid)] = peak
    require(all(len(pids) == 1 for pids in xids.values()), "terminal XID crossed runner")
    return peaks


def sum_edges(timelines):
    combined = Counter()
    for timeline in timelines:
        combined.update(timeline)
    return combined


def score_arm(folder, phase, label, config, remote):
    if label in public.PEERS:
        return public.score_arm(folder, phase, label, config, remote)
    # The public checker uses hot for the branch requiring actual app probe savings.
    # Every writer lane here uses that same app checkout configuration and file pins.
    score = public.score_arm(folder, phase, "hot", config, remote)
    with tarfile.open(folder / "raw.tar.gz") as raw:
        ledger = run.file_json(raw, "adapter/ledger.json")
        queue = run.file_json(raw, "adapter/queue-policy.json")
        traces = [json.loads(line.removeprefix("jobs_grouped_writer "))
                  for i in range(2)
                  for line in public.text(raw, f"adapter/runner-{i}.log").splitlines()
                  if line.startswith("jobs_grouped_writer ")]
        if POLICIES[label]:
            require(len(traces) == 2 and all(row["mode"] == POLICIES[label] for row in traces),
                    "actual writer mode or runner metrics missing")
            bulk = sum(sum(row["cohort_submissions_by_width"][1:]) for row in traces)
            require(bulk == queue["completion_cohort_queries"], "bulk SQL count disagrees with writers")
            require(sum(row["confirmed_free_claims"] for row in traces) == queue["admitted_queries"],
                    "actual dispatcher claim demand disagrees with SQL")
            if label == "hot":
                require(sum(row["refill_flushes"] for row in traces) > 0, "no demand-triggered handoff")
        else:
            require(not traces, "grouped control enabled the async writer")
    jobs = ledger["jobs"]
    effects = sorted(job["effect_committed_us"] for job in jobs)
    widths = Counter(Counter(job["settled_xid"] for job in jobs).values())
    return dict(score, committed_occupancy=occupancy(jobs), cohort_xid_widths=dict(widths),
                bulk_sql_calls=queue["completion_cohort_queries"], claim_sql_calls=queue["admitted_queries"],
                terminal_sql_elapsed_ms=sum(stmt["execution_ms"] for stmt in queue["queue_statements"]
                    if stmt["query"].startswith(("SELECT coalesce(mesh_jobs.settle_state(", "SELECT ordinal::text"))),
                middle_820_effect_us=effects[921] - effects[102], writer=traces)


def contrast(rows):
    candidate, a, b = (rows[key] for key in ("hot", "stock-a", "stock-b"))

    def savings(candidate, a, b):
        faster = min(a, b)
        saving = faster - candidate
        threshold = max(faster * .05, abs(a - b))
        return dict(candidate=candidate, controls=[a, b], saving=saving,
                    threshold=threshold, clears=saving > threshold)

    clocks = {name: savings(*(row["last_settlement_us"] - row[origin] for row in (candidate, a, b)))
              for name, origin in (("submit_us", "first_submit_us"), ("drain_us", "first_commit_us"))}
    earlier = min(row["last_effect_us"] - row["first_commit_us"] for row in (a, b)) - (
        candidate["last_effect_us"] - candidate["first_commit_us"])
    transactions = savings(*(row["terminal_xids"] for row in (candidate, a, b)))
    return dict(clocks=clocks, earlier_effect_us=earlier, terminal_transactions=transactions,
                advances=all(row["clears"] for row in clocks.values()) and earlier > 0
                and transactions["clears"])


def replay_block(path, block, config, remotes):
    require(run.read_json(path / "order.json") == block, "rotation order changed")
    return {f"{phase}-{label}": score_arm(folder, phase, label, config, run.REMOTE / remotes[label])
            for phase, label, _, folder in matrix.paths(path, config, block)}


def summarize(directory, config, schedule, pins):
    manifest = run.read_json(directory / "series.json")
    require(all(manifest[key] == value for key, value in
                dict(config=config, schedule=schedule, pins=pins).items()), "series bindings changed")
    blocks, failures, missing = {}, {}, []
    for block in schedule:
        group = block["group"]
        chosen = matrix.pick(directory, block, config, manifest["remotes"], failures)
        if chosen is None:
            missing.append(group)
            continue
        attempt, scores = chosen
        rows = {label: scores[f"measured-{label}"] for label in block["order"]}
        blocks[group] = dict(attempt=attempt, scores=scores,
                             gate=contrast(rows) if schedule == SCHEDULE else None)
    advances = schedule == SCHEDULE and not missing and all(block["gate"]["advances"] for block in blocks.values())
    return dict(blocks=blocks, missing=missing, failures=failures, advances=advances)


def grant(identifier):
    entries = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                         (public.QUEUE / "STATUS.md").read_text(), re.MULTILINE)
    mode, key, start, end = entries[-1]
    expected = "SETUP" if identifier.endswith("-setup") else "TIMING"
    require((mode, key) == (expected, identifier)
            and datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end),
            "active assignment required")
    require((public.QUEUE / f"{ID}.json").read_bytes() == REQUEST.read_bytes(),
            "request bytes differ from inbox")
    return dict(start=start, end=end, request_sha256=sha(REQUEST))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("check", "setup-spec", "bind", "timing", "replay", "peer-timing", "peer-replay"))
    parser.add_argument("inputs", nargs="*", type=Path)
    args = parser.parse_args()
    request = packet()
    if args.command == "check":
        local = HERE / "evidence/local"
        witness = run.read_json(local / "build.json")["witness"]["files"]["mesh"]
        validate_proofs({name: run.read_json(local / f"{name}.json") for name in PROOFS}, witness)
        faster = dict(first_submit_us=0, first_commit_us=100, last_settlement_us=1100,
                      last_effect_us=1000, terminal_xids=720)
        slower = dict(faster, last_settlement_us=1120, terminal_xids=730)
        winning = dict(faster, last_settlement_us=1000, last_effect_us=980, terminal_xids=600)
        rows = {"stock-a": faster, "stock-b": slower, "hot": winning}
        require(contrast(rows)["advances"], "positive gate regression")
        rows["hot"] = dict(winning, last_settlement_us=1050)
        require(not contrast(rows)["advances"], "strict 5% gate regression")
        print("Packet pins, local fault receipts, reverse orders, and pilot gate checked.")
        return 0
    if args.command == "setup-spec":
        (archive,) = args.inputs
        require(sha(archive) == request["source_archive_sha256"], "source archive changed")
        receipt = grant(ID + "-setup")
        attempt = f"trusted-jobs-product-{sha(archive)[:12]}"
        with tarfile.open(archive) as source:
            script = public.archive_files(source, "source/")(str(REPLAY / "native-setup.sh"))
        print(json.dumps(dict(receipt=receipt, attempt=attempt,
                              stdin_archive=str(archive.resolve()), script=script.decode(),
                              arguments=[attempt, BASE, sha(archive)],
                              remote_evidence=str(public.BASE / "runs" / attempt)),
                         indent=2, sort_keys=True))
        return 0
    archive, release, raw, *outputs = (path.resolve(strict=True) if i < 3 else path.resolve()
                                      for i, path in enumerate(args.inputs))
    binding = setup(archive, release, raw)
    if args.command == "bind":
        print(json.dumps(binding, indent=2, sort_keys=True))
        return 0
    require(archive.name == "source.tar.gz", "use the packet source.tar.gz")
    public.PRODUCT = str(archive.parent)
    peers = args.command.startswith("peer-")
    output = outputs[0]
    config = public.configurations(binding)
    labels = PEER_FIRST if peers else FIRST
    config["lanes"] = {label: config["lanes"].get(label, dict(engine="mesh", runners=2)) for label in labels}
    schedule = PEER_SCHEDULE if peers else SCHEDULE
    pins = dict(setup=binding, request_sha256=sha(REQUEST))
    identifier = ID + ("-peers" if peers else "-pilot")
    if peers:
        pilot_release, pilot_output = outputs[1:]
        decision = run.read_json(pilot_release)
        require(decision["id"] == ID + "-pilot" and decision["exit_status"] == 0, "pilot release required")
        pilot_config = dict(config, lanes={label: dict(engine="mesh", runners=2) for label in FIRST})
        require(summarize(pilot_output, pilot_config, SCHEDULE, pins)["advances"], "both pilot gates must pass")
    matrix.stages = stages
    matrix.replay_block = replay_block
    matrix.summarize = summarize
    matrix.grant = lambda _: grant(identifier)
    matrix.SERIES["equal"] = identifier
    if args.command.endswith("replay"):
        result = summarize(output, config, schedule, pins)
        print(json.dumps(result, indent=2, sort_keys=True))
        return bool(result["missing"])
    grant(identifier)
    dispatch_arm = run.dispatch_arm

    def dispatch_checked(path, remote, block, phase, engine):
        dispatch_arm(path, remote, block, phase, engine)
        folder = path / f"{phase}-{engine}"
        require(run.read_json(folder / "dispatch.json").get("exit_code") == 0,
                "native arm failed; preserve evidence and reconcile before retry")
        label = path.name.removeprefix(phase + "-")
        score_arm(folder, phase, label, config, remote)

    run.dispatch_arm = dispatch_checked
    output.parent.mkdir(parents=True, exist_ok=True)
    return matrix.dispatch(output, config, "equal", schedule, pins, [1, 2])


if __name__ == "__main__":
    sys.exit(main())
