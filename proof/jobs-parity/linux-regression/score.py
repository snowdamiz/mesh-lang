"""Apply the frozen Jobs parity limits to five same-artifact regression pairs."""

import argparse
import importlib.util
import json
from pathlib import Path

from pack import ROOT

spec = importlib.util.spec_from_file_location("jobs_parity", ROOT / "scripts/jobs-parity.py")
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)

TOPOLOGY = {"noop": (8, 64, 96), "slow": (1, 64, 68),
            "sparse": (1, 1, 5), "short": (1, 16, 20), "large": (8, 64, 96)}


def score(receipt):
    settings = json.loads((ROOT / "scripts/jobs-parity-profiles.json").read_text())
    assert len({group["mesh_elf_sha256"] for group in receipt["groups"]}) == 1
    assert len({group["guest_base_sha256"] for group in receipt["groups"]}) == 1
    assert len({group["kernel_sha256"] for group in receipt["groups"]}) == 1
    assert all(group["shared_component_sha256"] == receipt["groups"][0]["shared_component_sha256"]
               for group in receipt["groups"])
    profiles = {group["profile"] for group in receipt["groups"]}
    assert len({group["series"] for group in receipt["groups"]}) == 1
    if {group["series"] for group in receipt["groups"]} == {"canonical-oban-flags"}:
        assert all(group["oban_erl_flags"] == settings["oban_erl_flags"] for group in receipt["groups"])
        assert profiles == set(TOPOLOGY)
    result = {}
    for profile, (runners, slots, cap) in TOPOLOGY.items():
        if profile not in profiles:
            continue
        groups = [group for group in receipt["groups"] if group["profile"] == profile]
        pairs = []
        wins = 0
        for index, group in enumerate(groups):
            pair = {arm["metrics"]["engine"]: arm["metrics"] for arm in group["arms"]
                    if arm["phase"] == "measured"}
            assert pair.keys() == {"mesh", "oban"}
            wins += pair["mesh"]["jobs_per_second"] > pair["oban"]["jobs_per_second"]
            pairs.extend(dict(metrics, pair=index) for metrics in pair.values())
        harness.validate_pairs(pairs, settings["trial_pairs"], slots)
        summary = harness.profile_summary(pairs, profile, settings["workloads"][profile], settings,
                                          runners, slots, ("mesh", "oban"), True, True, None)
        assert summary["budgets"]["peak_connections"] == cap
        breaches = [
            f"{scope}.{metric}"
            for scope, values in summary.items() for metric, verdict in values.items()
            if isinstance(verdict, dict)
            and (verdict.get("within_limit") is False or verdict.get("within_budget") is False)
        ]
        result[profile] = {"pairs": [group["name"] for group in groups], "throughput_pair_wins": wins,
                           "scored_jobs": sum(trial["completed"] for trial in pairs),
                           "budget_breaches": breaches, "summary": summary}
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("receipt", type=Path)
    args = parser.parse_args()
    print(json.dumps(score(json.loads(args.receipt.read_text())), indent=2, sort_keys=True))
