"""Decompose the released middle-512 effect interval at independent write ranks."""
import json
from bisect import bisect_right

from peer_grant_progress import EVIDENCE, SEAL, load


def timeline(jobs):
    return {
        name: sorted(job[field] for job in jobs)
        for name, field in (
            ("entry", "entered_us"), ("first", "attempt_committed_us"),
            ("effect", "effect_committed_us"), ("terminal", "settled_us"),
        )
    }


def middle(times):
    first, effect = times["first"], times["effect"]
    spacing = [effect[k-1] - first[k-1] for k in (256, 768)]
    first_span = first[767] - first[255]
    effect_span = effect[767] - effect[255]
    assert min(spacing) >= 0 and effect_span == first_span + spacing[1] - spacing[0]
    return dict(first_span_us=first_span, effect_span_us=effect_span,
                spacing_256_us=spacing[0], spacing_768_us=spacing[1])


def replay():
    release = json.loads((EVIDENCE / "controller-release.json").read_bytes())
    assert release["exit_status"] == 0 and release["request_sha256"] == SEAL
    groups = json.loads((EVIDENCE / "scored-before-release.json").read_bytes())["groups"]
    rows = []
    for index in range(1, 6):
        block = EVIDENCE / "blocks" / f"{index:02}-noop"
        scores = json.loads((block / "reconciled.json").read_bytes())
        assert scores == groups[str(index)]
        timelines, origins, lanes, comparisons = {}, {}, {}, {}
        for lane in ("B", "A1", "A2", "river", "oban", "apalis"):
            score = scores[f"measured-{lane}"]
            jobs, tx, _ = load(block, lane, score)
            payloads = {j["job_key"]: j["payload_sha256"] for j in jobs}
            if lane == "B":
                reference = payloads
                claim_lower = sorted(j["claimed_us"] for j in jobs)
                claim_upper = sorted(min(j["claimed_us"]+999, j["entered_us"]) for j in jobs)
            assert payloads == reference
            timelines[lane] = timeline(jobs)
            origins[lane] = tx["submitted_us"]
            lanes[lane] = dict(**middle(timelines[lane]), raw_sha256=score["raw_sha256"])
        b = timelines["B"]
        for lane in ("A1", "A2", "river", "oban", "apalis"):
            cutoff = b["effect"][255] + lanes[lane]["effect_span_us"]
            counts = {stage: bisect_right(stamps, cutoff) for stage, stamps in b.items()}
            delta_first = lanes["B"]["first_span_us"] - lanes[lane]["first_span_us"]
            delta_effect = lanes["B"]["effect_span_us"] - lanes[lane]["effect_span_us"]
            comparisons[lane] = dict(
                mesh_counts_at_peer_middle_duration=counts,
                mesh_claim_stamps_by_cutoff_min=bisect_right(claim_upper, cutoff),
                mesh_claim_stamps_by_cutoff_max=bisect_right(claim_lower, cutoff),
                first_span_difference_us=delta_first, effect_span_difference_us=delta_effect,
                spacing_change_difference_us=delta_effect-delta_first,
                mesh_missing_first_writes_to_768=max(0, 768-counts["first"]),
                first_committed_pending_second=counts["first"]-counts["effect"],
                mesh_last_first_minus_peer_last_terminal_from_submit_us=(
                    b["first"][-1]-origins["B"] -
                    (timelines[lane]["terminal"][-1]-origins[lane])),
            )
        rows.append(dict(rotation=index, lanes=lanes, mesh_minus=comparisons))
    return rows


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
