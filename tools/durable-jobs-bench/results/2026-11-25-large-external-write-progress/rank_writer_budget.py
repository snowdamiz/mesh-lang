"""Bound scalar Execute elapsed inside the observed client's pre-BindComplete window."""

import argparse
from decimal import Decimal
from hashlib import sha256
import json
import math
from pathlib import Path
from statistics import median
import tarfile


RAW = "e4ca7e920525479719fdd5ec88a0a0e71a6c9540e7d63f137f46c6a862ec58e2"
RELEASE = "04e0e316bf3098a2f54891156442ab2227ad64fe98b99e4417e432c02b925520"
QUERY = "SELECT mesh_jobs.enqueue($1, $2::jsonb, $3, $4)::text AS receipt"
EDGES = ("writer_send", "writer_sent", "writer_bind_reply", "writer_ready")


def executor_spikes(statement, upper_ns):
    """Bound the elapsed saved by clipping only the slow SQL calls."""
    assert statement["stats_since"] == statement["minmax_stats_since"]
    low = statement["min_exec_time"]
    high = statement["max_exec_time"]
    count = len(upper_ns)
    total = statement["total_exec_time"]
    mean = total / count
    square_sum = count * (mean * mean + statement["stddev_exec_time"] ** 2)
    assert 0 < low <= mean < high
    upper = [Decimal(ns) / 1_000_000 for ns in upper_ns]
    # Accept one nanosecond of rounding when locating the largest execution.
    ranks = [i for i, wall in enumerate(upper, 1) if wall + Decimal("0.000001") >= high]
    assert ranks
    bands = {}
    for threshold in (5, 10, 15):
        assert low < threshold
        # For x <= threshold, (x-low)*(x-threshold) <= 0.
        # Above threshold its value is at most (high-low)*(x-threshold).
        moment = (square_sum - (low + threshold) * total + count * low * threshold) / (high - low)
        floor = max(0, high - threshold, total - count * threshold, moment)
        ceiling = min(total - count * low,
                      sum(max(0, min(wall, high) - threshold) for wall in upper))
        # Welford statistics and extrema were exported as decimal representations
        # of floating-point milliseconds. Round the reported bounds outwards.
        floor_us = max(0, math.floor(floor * 1000) - 1)
        ceiling_us = math.ceil(ceiling * 1000) + 1
        assert floor_us <= ceiling_us
        bands[threshold] = (floor_us, ceiling_us)
    return dict(ranks=ranks, max_ms=high, bands=bands)


def measure(archive, trace_name):
    arm = trace_name.split("/")[0]
    doc = json.load(archive.extractfile(f"{arm}/file-producer-budget.json"), parse_float=Decimal)
    before, after = (doc[key]["data"] for key in ("before", "after"))
    assert before["rows"] == []
    for key in ("stats_reset", "server_started", "dealloc", "track"):
        assert before[key] == after[key]
    stmt, = after["rows"]
    assert stmt["calls"] == stmt["rows"] == 64 and stmt["toplevel"] and stmt["query"] == QUERY
    assert stmt["dbid"] == str(doc["session"]["dbid"]) and stmt["userid"] == str(doc["session"]["userid"])
    elapsed_ns = math.floor(stmt["total_exec_time"] * 1_000_000)

    lines = archive.extractfile(trace_name).read().decode().splitlines()
    signature, pid, count = lines[0].split()
    assert signature == "file_rank_probe_v1" and int(pid) == doc["session"]["pid"]
    assert int(count) == len(lines) - 1 == 1093
    events = []
    client_marks = []
    previous = 0
    for line in lines[1:]:
        prefix, label, start, client_cpu, _, end = line.split()
        start, end = int(start), int(end)
        assert prefix == "file_rank" and previous <= start <= end
        previous = end
        if label in EDGES:
            events.append((label, start, end))
        if label in ("file_input", "prepare_start", "release_ready"):
            client_marks.append((label, int(client_cpu)))
    assert [point[0] for point in events] == list(EDGES) * 64
    assert [label for label, _ in client_marks] == ["file_input"] + ["prepare_start", "release_ready"] * 64
    assert all(a[1] <= b[1] for a, b in zip(client_marks, client_marks[1:]))
    starts = [cpu for label, cpu in client_marks if label == "prepare_start"]
    releases = [cpu for label, cpu in client_marks if label == "release_ready"]
    ingress_cpu = starts[0] - client_marks[0][1]
    between_cpu = [start - release for release, start in zip(releases[:-1], starts[1:], strict=True)]
    full_calls = []
    whole = send = pre_reply = tail = 0
    for i in range(0, len(events), 4):
        start, sent, bind, ready = events[i:i + 4]
        full_calls.append(ready[2] - start[1])
        whole += full_calls[-1]
        send += sent[2] - start[1]
        pre_reply += bind[2] - sent[1]
        tail += ready[2] - bind[1]
    overlap = max(0, elapsed_ns - send - tail)
    assert elapsed_ns <= whole and overlap <= pre_reply
    return arm, dict(ingress_cpu_ns=ingress_cpu, between_cpu_ns=between_cpu,
                     spikes=executor_spikes(stmt, full_calls), writer_wall_upper_ns=whole, executor_floor_ns=elapsed_ns,
                     wall_outside_executor_upper_ns=whole - elapsed_ns,
                     sent_to_bind_visible_upper_ns=pre_reply,
                     executor_within_sent_to_bind_lower_ns=overlap,
                     executor_share_lower_percent=100 * overlap / pre_reply)


def split_bind_budget(screen, measurements):
    """Partition the component contrast on one stock reference per block."""
    release = (screen / "controller-release.json").read_bytes()
    assert sha256(release).hexdigest() == "03e153105b0401fb2bfca56e2485def919f9148a795cebf298886d1803d2a59d"
    assert json.loads(release)["exit_status"] == 0
    raw = screen / "raw.tar.gz"
    assert sha256(raw.read_bytes()).hexdigest() == "f15829bdc4b28c64b673f8fcfc365a462bcf505458cad95b840cfb9bb94dea24"
    residuals = [value["wall_outside_executor_upper_ns"] for value in measurements.values()]
    print(f"FILE stock writer exchange residual upper bounds: {min(residuals)/1e6:.3f} to {max(residuals)/1e6:.3f} ms")
    print("block  stock     lane        wall_gain_ms  executor_gain_ms  other_gain_ms  other_needed_ms")
    with tarfile.open(raw) as archive:
        report = json.load(archive.extractfile("report.json"))
        assert report["component_eligibility"] and len(report["blocks"]) == 6
        floors = report["seal"]["full_submit_floor_us"]
        for index, (block, floor_us) in enumerate(zip(report["blocks"], floors, strict=True), 1):
            assert set(block) == {"stock_a", "stock_b", "split_json", "split_text"}
            executor = {}
            for lane, arm in block.items():
                label = f"split_{index}_{lane}"
                observed = json.load(archive.extractfile(f"stats-{label}.json"))
                detail = json.load(archive.extractfile(f"{label}.json"))
                assert observed["stats"] == arm["stats"] and detail["arm"] == arm
                assert detail["product"] == arm["commit"] and len(detail["rows"]) == arm["subxids"] == 64
                writer, = [row for row in observed["stats"] if row[0] == observed["writer"]]
                assert writer[1:3] == ["64", "64"]
                executor[lane] = Decimal(writer[3]) * 1_000_000
            ref = min(("stock_a", "stock_b"), key=lambda lane: block[lane]["enqueue_wall_ns"])
            spread = abs(block["stock_a"]["enqueue_wall_ns"] - block["stock_b"]["enqueue_wall_ns"])
            wall_floor = max(60_000_000, floor_us * 1000, spread)
            for lane in ("split_json", "split_text"):
                elapsed = block[ref]["enqueue_wall_ns"] - block[lane]["enqueue_wall_ns"]
                inside = executor[ref] - executor[lane]
                outside = elapsed - inside
                needed = max(0, wall_floor - inside)
                print(f"{index:5}  {ref:7}  {lane:10}", *(f"{v/1_000_000:14.3f}" for v in
                      (elapsed, inside, outside, needed)))


def main(folder, show_spikes, show_client, screen):
    release = (folder / "controller-release.json").read_bytes()
    assert sha256(release).hexdigest() == RELEASE and json.loads(release)["exit_status"] == 1
    raw = folder / "raw.tar.gz"
    assert sha256(raw.read_bytes()).hexdigest() == RAW
    native = json.loads((folder / "native-status.json").read_bytes())
    assert native["raw_sha256"] == RAW and native["exit_status"] == native["export_exit_status"] == 0
    with tarfile.open(raw) as archive:
        results = dict(measure(archive, name) for name in archive.getnames() if name.endswith("/rank-trace.txt"))
    assert len(results) == 12
    for group in range(1, 7):
        assert sorted(k.split("-", 2)[2] for k in results if k.startswith(f"g{group}-")) == ["on-a", "on-b"]
    if screen is not None:
        split_bind_budget(screen, results)
        return
    if show_client:
        print("arm             first_input_to_prepare  between_calls  ranks_2_to_16  ranks_17_to_64  (broker CPU-ms)")
        for arm, values in sorted(results.items()):
            between = values["between_cpu_ns"]
            row = (values["ingress_cpu_ns"], sum(between), sum(between[:15]), sum(between[15:]))
            print(f"{arm:15}", *(f"{ns / 1e6:14.3f}" for ns in row))
        return
    if show_spikes:
        print("arm              max_SQL_ms  possible_ranks  saved_ms_if_capped_at_5 / 10 / 15_ms")
        for arm, values in sorted(results.items()):
            spikes = values["spikes"]
            bands = [f"{lo/1000:.3f}–{hi/1000:.3f}" for lo, hi in spikes["bands"].values()]
            print(f"{arm:14} {spikes['max_ms']:10.3f}  {str(spikes['ranks']):15}", "  ".join(bands))
        return
    print("arm              executor_ms  residual_max_ms  overlap_min_ms  overlap_min_pct")
    rows = []
    for arm, values in sorted(results.items()):
        row = [values["executor_floor_ns"] / 1e6, values["wall_outside_executor_upper_ns"] / 1e6,
               values["executor_within_sent_to_bind_lower_ns"] / 1e6, values["executor_share_lower_percent"]]
        rows.append(row)
        print(f"{arm:15}", *(f"{x:14.3f}" for x in row))
    print("median         ", *(f"{median(x):14.3f}" for x in zip(*rows)))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--spikes", action="store_true")
    mode.add_argument("--client", action="store_true")
    mode.add_argument("--split-bind", type=Path, metavar="SCREEN",
                      help="compare the qualified split-bind screen with these FILE exchanges")
    args = parser.parse_args()
    main(args.evidence, args.spikes, args.client, args.split_bind)
