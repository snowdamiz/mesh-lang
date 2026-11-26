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


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exposure", type=Path)
    print(json.dumps(replay(parser.parse_args().exposure), indent=2))
