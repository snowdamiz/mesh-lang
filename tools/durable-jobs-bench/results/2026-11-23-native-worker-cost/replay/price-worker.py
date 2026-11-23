"""Price pre-handler native samples using the sealed ledger and exact waits."""

import argparse
from collections import Counter
from decimal import Decimal
from hashlib import sha256
import json
from pathlib import Path
import re
import sys
import tarfile


PREPARED = Path(__file__).resolve().parent.with_name("linode-dense-runtime-attribution")
sys.path.insert(0, str(PREPARED))
from reduce import CPU_HEADER, TOP_FRAME


FILE_FAULT = re.compile(r"^(filemap_fault|filemap_map_pages|do_read_fault)$")
ANON_FAULT = re.compile(r"^(do_anonymous_page|alloc_anon_folio|do_wp_page|wp_page_copy)$")
FAULT = re.compile(r"^(do_user_addr_fault|_?_?handle_mm_fault|do_fault|set_pte_range)$")
EXEC = re.compile(r"^(bprm_execve|exec_binprm|exec_mmap|load_elf_binary|do_execve.*)$")
CLONE = re.compile(r"^(copy_process|kernel_clone|clone3|alloc_thread_stack|pthread_create)(?:[.@].*)?$")
STACK_QUERY = re.compile(r"^(?:__)?pthread_getattr_np(?:[.@].*)?$")
MAPS = re.compile(r"^(show_map_vma|proc_pid_maps_open|pid_maps_open|show_map)$")
OFFSET = re.compile(r"\+0x[0-9a-f]+(?:/0x[0-9a-f]+)?$")


def samples(lines):
    record, frames = None, []
    for line in lines:
        if next_record := CPU_HEADER.match(line):
            if record:
                yield record, frames
            record = next_record
            frames = []
            line = record["leaf"]
        if record and (frame := TOP_FRAME.search(line)):
            symbol, image = frame.groups()
            frames.append((OFFSET.sub("", symbol), image))
    if record:
        yield record, frames


def category(frames, comm, leader):
    if not frames:
        return "unknown"
    kernel = [name for name, image in frames if "kernel.kallsyms" in image or "vmlinux" in image]
    user = [(name, image) for name, image in frames
            if "kernel.kallsyms" not in image and "vmlinux" not in image]
    names = [name for name, _ in frames]
    if any(STACK_QUERY.fullmatch(name) for name in names):
        return "pthread_getattr_np"
    if any(EXEC.fullmatch(name) for name in kernel):
        return "exec loader entry in kernel"
    if any(CLONE.fullmatch(name) for name in names):
        return "thread creation"
    if any(FILE_FAULT.fullmatch(name) for name in kernel):
        origin = Path(user[0][1]).name if user else None
        group = {"mesh": "from Mesh instruction", "ld-linux-x86-64.so.2": "from dynamic loader instruction"}
        return "file-backed fault " + group.get(origin, "with other or unknown caller")
    if any(ANON_FAULT.fullmatch(name) for name in kernel):
        return "anonymous or copy-on-write fault"
    if any(FAULT.fullmatch(name) for name in kernel):
        return "unresolved backing fault"
    if kernel and any(MAPS.fullmatch(name) for name in kernel):
        return "proc maps read with unresolved caller"
    if user and not kernel and Path(user[0][1]).name == "ld-linux-x86-64.so.2":
        return "dynamic loader user CPU"
    if leader and comm.startswith("mesh-jobs-"):
        return "inherited runner comm before worker image"
    return "other leader CPU" if leader else "other worker thread CPU"


def report(archive_path, reduced_path):
    reduced = json.loads(reduced_path.read_text())
    arm = reduced.get("arms", {}).get("03-mesh-on", {})
    result = {"archive_sha256": sha256(archive_path.read_bytes()).hexdigest(),
              "screens": reduced.get("observer_perturbation", {}).get("mesh", {}),
              "capture_issues": arm.get("capture_issues"),
              "matched_arm_rejection": arm.get("rejected")}
    waits = arm.get("exact_child_waits")
    if not waits:
        return result | {"pricing_unavailable": "no reconciled exact waits to join sample clocks and PIDs"}
    result["exact_wait_workers"] = waits["workers"]
    result["exact_lifetime_cpu_ms_per_job"] = waits["child_total_cpu_ms"] / waits["workers"]
    result["windows"] = {}
    offset = waits["clock_offset_ns"]
    uncertainty = waits["clock_uncertainty_ns"]
    by_pid = {row["pid"]: row for row in waits["jobs"]}
    with tarfile.open(archive_path, "r:gz") as archive:
        try:
            stacks = archive.extractfile("03-mesh-on/cpu.perf.txt").read().decode()
        except (KeyError, UnicodeDecodeError):
            return result | {"pricing_unavailable": "no usable call-chain export"}
        ledger = json.load(archive.extractfile("03-mesh-on/adapter/ledger.json"))
    entered = {row["worker_pid"]: row["entered_us"] * 1000 for row in ledger["jobs"]}
    if set(entered) != set(by_pid):
        raise ValueError("exact waits and ledger worker identities differ")
    usable = all(result["screens"].get(name, {}).get("within_diagnostic_screen")
                 for name in ("drain", "middle80")) and not any(
        issue.startswith("cpu profile unavailable or rejected") for issue in result["capture_issues"])
    for name, window in arm["windows"].items():
        periods, count, leafs = Counter(), Counter(), Counter()
        uncertain_edge, outside_wait = 0, 0
        if not window.get("cpu_profile", {}).get("samples"):
            continue
        start, end = (epoch_us * 1000 for epoch_us in window["epoch_us"])
        for record, frames in samples(stacks.splitlines()):
            pid = int(record["pid"])
            if pid not in by_pid:
                continue
            start_mono = int(Decimal(record["time"]) * 1_000_000_000)
            epoch = start_mono + offset
            if not start <= epoch <= end or epoch > entered[pid] + uncertainty:
                continue
            if not by_pid[pid]["spawn_ns"] <= start_mono <= by_pid[pid]["reap_ns"]:
                outside_wait += 1
                continue
            if not start + uncertainty <= epoch <= min(end, entered[pid]) - uncertainty:
                uncertain_edge += 1
                continue
            bucket = category(frames, record["comm"], pid == int(record["tid"]))
            count[bucket] += 1
            periods[bucket] += int(record["period"] or 0)
            if frames:
                leafs[bucket, frames[0][0], frames[0][1]] += 1
        denominator = waits["workers"] * 1_000_000
        result["windows"][name] = {
            "pre_handler_samples": count.total(),
            "uncertain_edge_samples": uncertain_edge,
            "outside_exact_wait_samples": outside_wait,
            "pricing_eligible": bool(usable and count and not outside_wait
                                     and all(ns > 0 for ns in periods.values())),
            "recorded_prices": [
                {"bucket": bucket, "samples": n, "sample_cpu_ms_per_job": periods[bucket] / denominator}
                for bucket, n in count.most_common()
            ],
            "top_bucket_leaves": [
                {"bucket": bucket, "symbol": symbol, "image": image, "samples": n}
                for (bucket, symbol, image), n in leafs.most_common(50)
            ],
        }
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("reduced", type=Path)
    args = parser.parse_args()
    print(json.dumps(report(args.archive, args.reduced), indent=2, allow_nan=False))
