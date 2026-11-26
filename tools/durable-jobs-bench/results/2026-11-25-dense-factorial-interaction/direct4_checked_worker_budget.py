"""Price recorded Direct4 handler residence against the warm River middle interval."""
import argparse
import hashlib
import json
import runpy
import tarfile
from collections import Counter, defaultdict
from pathlib import Path
from statistics import median

from direct4_peer_deadlines import warm_river


def pinned(path, expected):
    assert hashlib.sha256(path.read_bytes()).hexdigest() == expected, path


def saturated_episodes(jobs, begin, end):
    """Contiguous all-four reply intervals with one or two fresh jobs provably unpicked."""
    events = defaultdict(Counter)
    for job in jobs:
        for name, left, right in (("reply", "sent1", "ready1"), ("reply", "sent2", "ready2"),
                                  ("fresh", "pub_after", "pick_lower")):
            start, stop = max(begin, job[left]), min(end, job[right])
            if start < stop:
                events[start][name] += 1
                events[stop][name] -= 1

    live = Counter()
    opened = {}
    spans = {1: [], 2: []}
    for stamp in sorted(events):
        live.update(events[stamp])
        assert 0 <= live["reply"] <= 4 and live["fresh"] >= 0
        for width in spans:
            eligible = live["reply"] == 4 and live["fresh"] >= width
            if eligible and width not in opened:
                opened[width] = stamp
            elif not eligible and width in opened:
                spans[width].append(stamp - opened.pop(width))
    assert not opened and not any(live.values())
    assert sum(spans[2]) <= sum(spans[1]) <= end-begin
    return {width: dict(episodes=len(durations), total_ns=sum(durations),
                        longest_ns=max(durations, default=0),
                        after_half_ms_ns=sum(max(0, d - 500_000) for d in durations),
                        after_one_ms_ns=sum(max(0, d - 1_000_000) for d in durations))
            for width, durations in spans.items()}


def base_reply_windows(jobs, begin, end, include=None):
    """Preserve the identities of all four original waiters at each common interval."""
    changes = defaultdict(list)
    for job in jobs:
        for seq in (1, 2):
            if include is not None and not include(job, seq):
                continue
            start = max(begin, job[f"sent{seq}"])
            stop = min(end, job[f"ready{seq}"])
            if start < stop:
                changes[start].append((1, job["worker"], job["id"]))
                changes[stop].append((-1, job["worker"], job["id"]))
    waiting = {}
    windows = []
    previous = begin
    for stamp, changes_at_stamp in sorted(changes.items()):
        if len(waiting) == 4:
            windows.append((previous, stamp, frozenset(waiting.values())))
        for direction, worker, identifier in sorted(changes_at_stamp):
            if direction < 0:
                assert waiting.pop(worker) == identifier
            else:
                assert worker not in waiting
                waiting[worker] = identifier
        previous = stamp
    assert not waiting
    return windows


def sampled_reply_coverage(jobs, begin, end, width, offset):
    """Project a one-quarter duty mask onto the older checked-return trace."""
    period = 4 * width
    possible = set()
    certain = set()
    for job in jobs:
        for seq in (1, 2):
            # The new observer selects after checkout, before sending. The older
            # recorder brackets that edge with its POOL and SEND events.
            lo, hi = job[f"pool{seq}"] + offset, job[f"send{seq}"] + offset
            assert lo <= hi
            key = job["id"], seq
            phase = lo % period
            if phase < width or hi - lo >= period - phase:
                possible.add(key)
            if phase < width and hi - lo < width - phase:
                certain.add(key)
    assert certain <= possible
    result = {}
    for label, selected in (("certain", certain), ("possible", possible)):
        spans = base_reply_windows(jobs, begin, end, lambda job, seq: (job["id"], seq) in selected)
        result[label] = dict(operations=len(selected), four_way_ns=sum(b-a for a, b, _ in spans))
    assert result["certain"]["four_way_ns"] <= result["possible"]["four_way_ns"]
    return result


def sampling_periods(jobs, begin, end):
    result = {}
    for width in (2_000_000, 4_000_000, 8_000_000, 16_000_000):
        # Eight phases at each duty cycle, including both owner offsets.
        cuts = [sampled_reply_coverage(jobs, begin, end, width, phase * width // 2)
                for phase in range(8)]
        result[width] = {
            field: [min(c["certain"][field] for c in cuts), max(c["possible"][field] for c in cuts)]
            for field in ("operations", "four_way_ns")
        }
    return result


def pickups_before_first_reply(jobs, windows):
    """Replay the first two known publications during each distinct four-worker wait."""
    ordered = sorted(jobs, key=lambda job: (job["pub_after"], job["id"]))
    results = {}
    for delay in (0, 500_000, 1_000_000):
        counts = Counter()
        unique = defaultdict(set)
        for left, right, _ in windows:
            chosen = []
            for job in ordered:
                pickup = max(left, job["pub_after"]) + delay
                if pickup >= right:
                    break
                if pickup < job["pick_lower"]:
                    chosen.append((job, pickup))
                    if len(chosen) == 2:
                        break
            if len(chosen) == 2:
                counts["two_pickups"] += 1
            for edge in ("sent1", "checked1", "checked2"):
                fits = [j["id"] for j, pickup in chosen if pickup + j[edge] - j["pick"] <= right]
                counts["both_" + edge] += len(fits) == 2
                unique[edge].update(fits)
        results[delay] = dict(counts=counts, distinct_jobs={k: len(v) for k, v in unique.items()})
    return results


def spare_schedule(jobs, windows, width, delay, first_checkout=0, later_checkout=0):
    """Keep borrowed workers charged through release and remove moved jobs' old waits."""
    ordered = sorted(jobs, key=lambda job: (job["pub_after"], job["id"]))
    available = [0] * width
    moved = {}
    for left, right, waiters in windows:
        if any(identifier in moved for identifier in waiters):
            continue
        # One launch per spare per window, oldest available spare first.
        for slot in sorted(range(width), key=available.__getitem__):
            earliest = max(left, available[slot])
            for job in ordered:
                pickup = max(earliest, job["pub_after"]) + delay
                if pickup >= right:
                    break
                if job["id"] in moved or pickup >= job["pick_lower"]:
                    continue
                assert job["id"] not in waiters and pickup < job["pick"]
                checkout = later_checkout if moved else first_checkout
                finish = pickup + checkout + job["release"] - job["pick"]
                available[slot] = finish
                moved[job["id"]] = dict(advance=job["pick"]-pickup-checkout, residence=finish-pickup,
                                        carry=max(0, finish-right),
                                        checked=pickup+checkout+job["checked2"]-job["pick"] <= right)
                break
    return moved


def resident_spares(jobs, windows, width, delay):
    moves = spare_schedule(jobs, windows, width, delay)
    residence = sum(move["residence"] for move in moves.values())
    carry = sum(move["carry"] for move in moves.values())
    fully_checked = sum(move["checked"] for move in moves.values())
    assert 0 <= carry <= residence and fully_checked <= len(moves)
    return dict(distinct_jobs=len(moves), both_writes_checked_before_base_reply=fully_checked,
                residence_ns=residence, residence_after_base_reply_ns=carry)


def credit_crossings(jobs, moves, alignment_ns):
    """Shift each actual settlement by its last member's change in return time."""
    cohorts = defaultdict(list)
    stamps = defaultdict(set)
    for job in jobs:
        cohorts[job["settled_xid"]].append(job)
        stamps[job["pid"]].add(job["claim"])
    earlier = earlier_rows = 0
    crossed_jobs, crossed_stamps = set(), set()
    possible_jobs, possible_stamps = set(), set()
    advances = []
    for cohort in cohorts.values():
        pid, = {job["pid"] for job in cohort}
        before = max(job["returned"] for job in cohort)
        after = max(job["returned"] - (moves[job["id"]]["advance"] if job["id"] in moves else 0)
                    for job in cohort)
        advance = before - after
        if advance <= 0:
            continue
        earlier += 1
        earlier_rows += len(cohort)
        advances.append(advance)
        for job in cohort:
            credit = job["credit"]
            for stamp in stamps[pid]:
                lower, upper = stamp-alignment_ns, stamp+1_000_000+alignment_ns
                if credit-advance < upper and credit > lower:
                    possible_jobs.add(job["id"])
                    possible_stamps.add((pid, stamp))
                if credit-advance < lower and credit > upper:
                    crossed_jobs.add(job["id"])
                    crossed_stamps.add((pid, stamp))
    assert crossed_jobs <= possible_jobs and crossed_stamps <= possible_stamps
    return dict(earlier_cohorts=earlier, earlier_rows=earlier_rows,
                median_earlier_ns=median(advances) if advances else 0,
                max_earlier_ns=max(advances, default=0),
                distinct_credits_crossing_stamp=len(crossed_jobs), distinct_stamps_crossed=len(crossed_stamps),
                possible_credits=len(possible_jobs), possible_stamps=len(possible_stamps))


def fixed_publication_ranks(jobs, owners, start, end, alignment_ns):
    """Move recorded writes of spare-picked jobs, leaving every other job at its observed times."""
    lanes = []
    for pid in owners:
        own = [job for job in jobs if job["pid"] == pid]
        lanes.append((own, base_reply_windows(own, start, end)))
    ranks = (64, 256, 512, 768, 960, 1024)
    original = sorted(job["record2"] for job in jobs)
    cases = {}
    for first, later in ((0, 0), (357_000, 0), (6_709_000, 0), (6_709_000, 6_709_000)):
        moves = {}
        for own, windows in lanes:
            selected = spare_schedule(own, windows, 1, 500_000, first, later)
            assert moves.keys().isdisjoint(selected)
            moves.update(selected)
        records = [job["record2"] - moves[job["id"]]["advance"] if job["id"] in moves else job["record2"]
                   for job in jobs]
        records.sort()
        cases[f"{first}/{later}"] = dict(moved_jobs=len(moves),
                                         moved_later=sum(move["advance"] < 0 for move in moves.values()),
                                         effect_advance_ns={i: original[i-1]-records[i-1] for i in ranks},
                                         credit_crossings=credit_crossings(jobs, moves, alignment_ns))
    return cases


def publication_floor(jobs, workers):
    """Earliest possible final effect with fixed publication and pickup-to-effect durations."""
    earliest = 0
    binding = {}
    for pid in {job["pid"] for job in jobs}:
        work = 0
        newest = sorted((j for j in jobs if j["pid"] == pid), key=lambda j: -j["pub_before"])
        for count, job in enumerate(newest, 1):
            published = job["pub_before"]
            service = job["record2"] - job["pick"]
            assert service > 0
            work += service
            capacity = (work + workers - 1) // workers
            finish = published + max(service, capacity)
            if finish > earliest:
                earliest = finish
                binding = dict(pid=pid, job=job["key"], jobs_from_cut=count,
                               constraint="single_job" if service >= capacity else "worker_capacity")
    last = max(job["record2"] for job in jobs)
    assert earliest <= last
    return dict(maximum_advance_ns=last-earliest, **binding)


def worker_budget(arm, phase, digest, decode, peers):
    raw = arm / "raw.tar.gz"
    pinned(raw, digest)
    assert json.loads((arm / "dispatch.json").read_bytes())["exit_code"] == 0
    with tarfile.open(raw) as archive:
        def adapter(name):
            return archive.extractfile("adapter/" + name).read()

        ledger = json.loads(adapter("ledger.json"))
        assert (ledger["phase"], ledger["slots"], ledger["runners"]) == (phase, 64, 2)
        broker, *runners = json.loads(adapter("process-identity.json"))
        assert len(runners) == 2
        assert all(proc["executable_sha256"] == "806f8022a0755a62661e9f05a8c91709a9b4147872c2616781fd835626dbb06e"
                   for proc in (broker, *runners))
        trace = {}
        for i, proc in enumerate(runners):
            pid = proc["pid"]
            logs = (adapter(f"runner-{i}.trace-{pid}-{lane}.log") for lane in (0, 1, 2, 3, 255))
            trace[pid] = decode["frames"](b"".join(logs), pid, True)
    txn, = ledger["transactions"]
    jobs, _, windows, alignment = decode["join"](ledger["jobs"], trace, txn)
    assert len(jobs) == len({job["id"] for job in jobs}) == 1024
    begin, end = windows["middle_512"]
    slack = alignment["alignment_allowance_ns"]

    def duration(start, stop):
        return max(0, min(end, stop) - max(begin, start))

    slowest = max(peer["phases"]["middle_us"] for peer in peers.values()) * 1000
    owners = []
    for pid in sorted(trace):
        own = [job for job in jobs if job["pid"] == pid]
        def area(start, stop):
            return sum(duration(job[start], job[stop]) for job in own)

        handler = area("start", "release")
        assert handler <= 4 * (end - begin)
        replies = []
        for n in (1, 2):
            sent, ready, record = f"sent{n}", f"ready{n}", f"record{n}"
            total = area(sent, ready)
            post = sum(duration(max(job[sent], job[record] + slack), job[ready]) for job in own)
            assert 0 <= post <= total
            replies.append(dict(send_to_ready_ns=total, definitely_after_commit_ns=post))
        assert sum(r["send_to_ready_ns"] for r in replies) <= handler
        reply_windows = base_reply_windows(own, begin, end)
        writes = sorted((j[f"pool{n}"], j[f"send{n}"], j[f"sent{n}"], j[f"ready{n}"], j["worker"])
                        for j in own for n in (1, 2))
        first = writes[:4]
        startup = dict(first_four_pool_to_send_ns=[send-pool for pool, send, *_ in first],
                       distinct_workers=len({write[4] for write in first}),
                       all_sent_before_first_ready=max(write[2] for write in first) < min(write[3] for write in first),
                       later_median_pool_to_send_ns=median(send-pool for pool, send, *_ in writes[4:]))
        sample_offset = next(i for i, proc in enumerate(runners) if proc["pid"] == pid) * 4_000_000
        owners.append(dict(pid=pid, effects=sum(begin < j["record2"] <= end for j in own),
                           checkout_startup=startup,
                           reply_window_ns=sum(b-a for a, b, _ in reply_windows),
                           sample_offset_ns=sample_offset,
                           sampled_reply_bounds=sampled_reply_coverage(own, begin, end, 2_000_000, sample_offset),
                           sampled_eight_phase_range=sampling_periods(own, begin, end),
                           middle_handler_ns=handler, saturated=saturated_episodes(own, begin, end),
                           before_base_reply=pickups_before_first_reply(own, reply_windows),
                           resident_spares={delay: {n: resident_spares(own, reply_windows, n, delay) for n in (1, 2)}
                                            for delay in (0, 500_000, 1_000_000)},
                           other_than_handler_ns=4*(end-begin)-handler,
                           reply_intervals=replies, published_before_pick_ns=area("pub_after", "pick_lower"),
                           workers_at_slowest_peer_cadence=handler / slowest,
                           five_worker_residence_excess_ns=max(0, handler - 5*slowest)))
    assert sum(o["effects"] for o in owners) == 512
    return dict(phase="instrumented warmup" if phase == "warmup" else phase, middle_ns=end-begin, owners=owners,
                fixed_publication_ranks=fixed_publication_ranks(jobs, trace, windows["first_256"][0], windows["last_256"][1], slack),
                final_effect_bound={n: publication_floor(jobs, n) for n in (4, 5, 6, 1024)},
                warm_river_middle_us={g: p["phases"]["middle_us"] for g, p in peers.items()},
                alignment_allowance_ns=slack)


def checked_worker_budget(evidence, decoder, measured):
    pinned(decoder, "6891d64914ff43fbabd76c5762349395a0f487b55dda8ccdb5e09f9e1b5ae91e")
    decode = runpy.run_path(str(decoder))
    peers = warm_river()
    release = evidence / "controller/release.json"
    if measured:
        return measured_budgets(evidence, release, decode, peers)
    pinned(release, "65115b81c78279e68594eaf98346ab56e8e057ff0232c1e3f1aef89ab5e6b3f8")
    assert json.loads(release.read_bytes())["exit_status"] == 1
    arm = evidence / "blocks/01-noop/warmup-on/warmup-mesh"
    return worker_budget(arm, "warmup", "ec73769de866fa4e808464003ac3a9f44bbfca937133a5552afe05f7c881badc",
                         decode, peers)


def measured_budgets(evidence, release, decode, peers):
    verification = evidence / "verified.json"
    pinned(verification, "4f310f20baa40a50dcc2ee5010c91db664f7523ed54e29e9ff18590b12846a60")
    report = json.loads(verification.read_bytes())
    pinned(release, report["release_sha256"])
    assert json.loads(release.read_bytes())["exit_status"] == 0
    assert report["request_sha256"] == "0268c3ec3b678a601ac074a2dfa1b1a083cd21cc1ce46c3dbe9f08b4bc7ef75c"
    assert report["complete"] and not report["errors"]
    blocks = {}
    for index in (1, 2):
        block = f"blocks/{index:02}-noop"
        score = report["arms"][block]["measured"]["on"]
        arm = evidence / block / "measured-on/measured-mesh"
        budget = worker_budget(arm, "measured", score["raw_sha256"], decode, peers)
        assert budget["middle_ns"] == score["middle_512_us"] * 1000
        blocks[block] = budget
    return dict(verdict=report["verdict"], release_sha256=report["release_sha256"], blocks=blocks)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path)
    parser.add_argument("decoder", type=Path)
    parser.add_argument("--measured", action="store_true", help="read both measured ON arms of the settlement successor")
    args = parser.parse_args()
    print(json.dumps(checked_worker_budget(args.evidence, args.decoder, args.measured), indent=2))
