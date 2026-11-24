#!/usr/bin/env python3
"""Reduce the untimed baseline claim snapshots and dispatcher slack samples."""
from collections import Counter, defaultdict
import json
from pathlib import Path
import sys

from campaign import arm_result, distribution, read


def analyze(folder):
    arm_result(folder, "probe")
    ledger = read(folder / "ledger.json")
    start = ledger["transactions"][0]["committed_us"]
    finish = max(job["entered_us"] for job in ledger["jobs"])
    lines = (folder / "runner-0.log").read_text().splitlines()
    origin, = (int(line.split()[1]) for line in lines if line.startswith("refill_probe_origin "))
    events = []
    for line in lines:
        if line.startswith("refill_probe "):
            _, stamp, kind, active, reserved, detail = line.split()
            events.append((origin + int(stamp), kind, int(active), int(reserved), int(detail)))
    assert events and all(a[0] <= b[0] for a, b in zip(events, events[1:]))
    current, claims, samples = None, [], []
    for at, kind, active, reserved, detail in events:
        if kind == "sample":
            samples.append((at, active, reserved))
        elif kind == "request":
            assert current is None and active + reserved == detail == 16
            current = dict(request_us=at, requested=reserved, active_at_request=active)
            samples.append((at, active, reserved))
        else:
            assert current is not None and reserved == current["requested"]
            if kind in ("sql_begin", "sql_end"):
                current[kind + "_us"] = at
            else:
                current.update(reply_us=at, active_at_reply=active, granted=detail, kind=kind)
                assert 0 <= detail <= reserved and active + reserved <= 16
                current["newly_free_during_turn"] = current["active_at_request"] - active
                assert current["newly_free_during_turn"] >= 0
                # The next sample follows confirmed launch. Room freed in a later
                # pass and hidden by a concurrent launch remains interval-censored.
                claims.append(current)
                current = None
    hot = [c for c in claims if c["reply_us"] >= start and c["request_us"] <= finish]
    assert sum(c["granted"] for c in hot) == 128
    hot_and_tail = [c for c in claims if c["reply_us"] >= start]
    refills = [c for c in hot if c["granted"]][1:]
    slices = defaultdict(int)
    slack = defaultdict(int)
    for (left, active, reserved), (right, _, _) in zip(samples, samples[1:]):
        duration = max(0, min(right, finish) - max(left, start))
        if not duration:
            continue
        assert active + reserved <= 16
        state = "pending" if reserved else "unreserved"
        slices[state] += duration
        slack[state] += duration * (16 - active - reserved)
        slack["all_free_local_slots"] += duration * (16 - active)
    def spread(field_start, field_end):
        return distribution(c[field_end] - c[field_start] for c in refills)

    return dict(
        scope="one untimed combined zero-hold arm; sampled intervals between changed dispatcher snapshots",
        milestone_span_us=finish - start, probe_origin_unix_us=origin,
        reservation_width=Counter(c["requested"] for c in hot), grant_width=Counter(c["granted"] for c in hot),
        active_at_request=Counter(c["active_at_request"] for c in refills),
        empty_turns_before_last_entry=sum(not c["granted"] for c in hot),
        empty_turns_through_stop=sum(not c["granted"] for c in hot_and_tail),
        errors=sum(c["kind"] == "error" for c in hot_and_tail),
        contention_hints=sum(c["kind"] == "contention" for c in hot_and_tail),
        underfilled_nonempty_before_last_entry=sum(0 < c["granted"] < c["requested"] for c in hot),
        new_free_before_reply_counts=Counter(c["newly_free_during_turn"] for c in refills),
        small_refill_open_three_by_reply=sum(c["requested"] < 3 and 16 - c["active_at_reply"] >= 3
                                             for c in refills),
        work_refill_turns=len(refills),
        refill_request_to_sql_us=spread("request_us", "sql_begin_us"),
        refill_sql_session_us=spread("sql_begin_us", "sql_end_us"),
        refill_sql_to_dispatcher_us=spread("sql_end_us", "reply_us"),
        sum_refill_sql_session_us=sum(c["sql_end_us"] - c["sql_begin_us"] for c in refills),
        sampled_wall_us=dict(slices), sampled_free_slot_us=dict(slack),
        mean_sampled_unassigned_slack_slots=(slack["pending"] + slack["unreserved"]) / (finish - start),
        all_requests=claims,
    )


if __name__ == "__main__":
    print(json.dumps(analyze(Path(sys.argv[1])), indent=2, sort_keys=True))
