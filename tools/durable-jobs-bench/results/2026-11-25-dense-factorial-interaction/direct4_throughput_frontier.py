"""Price 20% more throughput with the last 768 effects at historical River cadence."""
import argparse
import json
from pathlib import Path

from direct4_handler_coverage import controls
from direct4_peer_deadlines import clocks, warm_river, write_phases


def replay(exposure):
    peers = warm_river()
    rows = []
    for control, score, jobs, txn in controls(exposure):
        mesh = clocks(jobs, txn)
        assert all(score[key] == value for key, value in mesh.items())
        phases = write_phases(jobs, txn)
        enqueue = txn["committed_us"] - txn["submitted_us"]
        assert mesh["submit_us"] == enqueue + mesh["drain_us"]
        for group, river in peers.items():
            peer_phases = river["phases"]
            late = peer_phases["middle_us"] + peer_phases["last_256_us"]
            drain = phases["first_256_us"] + late + phases["terminal_tail_us"]
            assert drain == mesh["drain_us"] - phases["middle_us"] - phases["last_256_us"] + late
            assert river["drain_us"] == peer_phases["first_256_us"] + late + peer_phases["terminal_tail_us"]
            # Equal job counts: 1.20 times throughput permits 5/6 of elapsed time.
            allowed_enqueue = river["submit_us"] * 5 // 6 - drain
            rows.append(dict(control=control, mesh_sha256=score["raw_sha256"], river_group=group,
                             river_sha256=river["raw_sha256"], modeled_drain_us=drain,
                             drain_margin_us=river["drain_us"] * 5 // 6 - drain,
                             allowed_enqueue_us=allowed_enqueue, enqueue_us=enqueue,
                             required_enqueue_saving_us=enqueue - allowed_enqueue,
                             matched_enqueue_margin_us=allowed_enqueue - (river["submit_us"] - river["drain_us"])))
    assert len({(r["mesh_sha256"], r["river_sha256"]) for r in rows}) == len(rows) == 36
    return rows


def rank_768(exposure):
    """Read both ledgers at the latest warm River rank-768 effect clock."""
    peers = warm_river()
    deadlines = {group: row["phases"]["first_256_us"] + row["phases"]["middle_us"]
                 for group, row in peers.items()}
    group = max(deadlines, key=deadlines.get)
    cutoff = deadlines[group]
    rows = []
    for name, score, jobs, txn in controls(exposure):
        origin = txn["committed_us"]
        stamps = sorted(job["claimed_us"] - origin for job in jobs)
        first = sorted(job["attempt_committed_us"] - origin for job in jobs)
        effects = sorted(job["effect_committed_us"] - origin for job in jobs)
        # A millisecond claim stamp represents a bin. Count ambiguous claims
        # as available when bounding the improvement on this grant schedule.
        stamped_upper = sum(stamp <= cutoff for stamp in stamps)
        effect_count = sum(effect <= cutoff for effect in effects)
        assert effect_count <= stamped_upper and first[767] <= effects[767]
        rows.append(dict(control=name, mesh_sha256=score["raw_sha256"],
                         claims_possibly_stamped=stamped_upper, effects_committed=effect_count,
                         additional_claims_needed=max(0, 768 - stamped_upper),
                         claim_768_lag_floor_us=stamps[767] - cutoff,
                         first_write_768_lag_us=first[767] - cutoff,
                         first_to_second_rank_gap_us=effects[767] - first[767]))
    assert len({row["mesh_sha256"] for row in rows}) == 6
    return dict(river_group=group, river_sha256=peers[group]["raw_sha256"],
                river_rank_768_us=cutoff, controls=rows)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exposure", type=Path)
    parser.add_argument("--rank-768", action="store_true",
                        help="locate Mesh first writes and grants at River's 768th effect")
    args = parser.parse_args()
    print(json.dumps(rank_768(args.exposure) if args.rank_768 else replay(args.exposure), indent=2))
