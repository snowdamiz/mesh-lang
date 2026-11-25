"""Replay each historical series and quantify the EXTERNAL break-even thresholds."""

import json
from statistics import median

import matched as c


def historical():
    plan = c.configure()
    c.original.timing_plan = c.prior["plan"]
    default = c.original.scores(c.session_document(), c.RECOVERED, c.DEFAULT)
    assert default == c.read(c.DEFAULT.with_name(c.DEFAULT.name + "-scores.json"))
    public = c.read(c.EXTERNAL / "large-external.inventory.json")
    root = c.EXTERNAL / "large-external"
    expected = c.read(c.SESSION / "evidence/public-baselines.json")["public"]["large-external"]["blocks"]
    external = []
    assert {root / "blocks" / row["block"] for row in expected} == {
        p.parent for p in root.glob("blocks/*/reconciled.json")
    }
    for row in expected:
        block = public["recipe"]["blocks"][row["group"] - 1]
        path = root / "blocks" / row["block"]
        replayed = c.replay_block(public, path, block, matched=False)
        for lane, arm in row["arms"].items():
            assert all(replayed["clocks"][lane][key] == arm[key] for key in c.CLOCKS)
            assert replayed["receipts"]["measured"][lane]["raw_sha256"] == arm["raw_sha256"]
        clocks = replayed["clocks"]
        gaps = {peer: {key: round(clocks["product"][key] - clocks[peer][key], 3) for key in c.CLOCKS}
                for peer in c.PEERS}
        assert gaps == row["product_minus_peer_ms"]
        external.append(dict(group=row["group"], clock_gap_ms=gaps,
                             receipts=replayed["receipts"]))

    thresholds = {}
    for peer in c.PEERS:
        gaps = {key: [row["clock_gap_ms"][peer][key] for row in external] for key in c.CLOCKS}
        deficits = [max(0, gap) for gap in gaps["submit_ms"]]
        thresholds[peer] = dict(
            inline_minus_peer_ms={key: c.distribution(values) for key, values in gaps.items()},
            inline_submit_wins=sum(gap < 0 for gap in gaps["submit_ms"]),
            saving_to_tie_each_submit_pair_ms=deficits,
            constant_submit_saving_to_tie_worst_pair_ms=max(deficits),
            median_post_commit_gap_if_only_enqueue_changes_ms=median(gaps["drain_ms"]),
        )
    return dict(
        workload=dict(jobs=64, payload_bytes_each=524288, runners=8, slots=64, cpus=[0, 1], cap=96),
        default_file=default["summary"]["file"],
        default_faster_inline_minus_file_ms={key: c.distribution(
            [r["faster_duplicate_minus_file_ms"][key] for r in default["groups"]]) for key in c.CLOCKS},
        historical_external=dict(groups=external, thresholds=thresholds),
        conditional="Thresholds freeze each released EXTERNAL inline drain and its original peer time. "
                    "A submit threshold is the enqueue reduction needed to tie under those conditions. "
                    "The default FILE effects are independent observations. No effects are combined.",
        planned_series=dict(blocks=plan["recipe"]["blocks"], qualification=plan["session_qualification"]),
    )


if __name__ == "__main__":
    def offline(*args, **kwargs):
        raise AssertionError("historical replay attempted a host call")

    c.run.ssh = offline
    c.packet()
    print(json.dumps(historical(), indent=2, sort_keys=True, allow_nan=False))
