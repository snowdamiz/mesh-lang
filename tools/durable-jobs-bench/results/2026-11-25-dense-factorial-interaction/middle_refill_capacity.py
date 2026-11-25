"""Compare durable capacity after aligning each engine at its 256th effect."""
import json

from peer_grant_progress import EVIDENCE, SEAL, load, mesh_at


def effect_times(jobs):
    return sorted(job["effect_committed_us"] for job in jobs)


def counts(jobs, cutoff):
    return {
        "effects": sum(job["effect_committed_us"] <= cutoff for job in jobs),
        "settled": sum(job["settled_us"] <= cutoff for job in jobs),
    }


def compare(jobs, peer_jobs):
    mesh = effect_times(jobs)
    peer = effect_times(peer_jobs)
    duration = peer[767] - peer[255]
    cutoff = mesh[255] + duration
    state = counts(jobs, cutoff)
    stamps = mesh_at(jobs, cutoff)
    # Even if all 64 seats hold effect-complete jobs, the rest must have
    # settled before Mesh could deliver its 768th distinct effect.
    required = 768 - 64
    return dict(peer_middle_us=duration, mesh_middle_us=mesh[767] - mesh[255],
                mesh_at_rank_256=counts(jobs, mesh[255]), mesh_at_cutoff=state,
                peer_at_rank_768=counts(peer_jobs, peer[767]),
                mesh_granted_at_cutoff_bounds=[1024 - stamps["unstamped_upper"],
                                               1024 - stamps["unstamped_lower"]],
                finished_effects_awaiting_settlement=state["effects"] - state["settled"],
                additional_settlements_needed=max(0, required - state["settled"]),
                additional_effects_needed_even_for_704_settlements=max(0, required - state["effects"]))


def replay():
    release = json.loads((EVIDENCE / "controller-release.json").read_bytes())
    assert release["exit_status"] == 0 and release["request_sha256"] == SEAL
    series = json.loads((EVIDENCE / "scored-before-release.json").read_bytes())["groups"]
    output = {}
    for index in range(1, 6):
        block = EVIDENCE / "blocks" / f"{index:02}-noop"
        scores = json.loads((block / "reconciled.json").read_bytes())
        assert scores == series[str(index)]
        jobs, _, _ = load(block, "B", scores["measured-B"])
        inputs = {job["job_key"]: job["payload_sha256"] for job in jobs}
        row = {"mesh_raw_sha256": scores["measured-B"]["raw_sha256"]}
        for peer in ("river", "oban"):
            score = scores["measured-" + peer]
            others, _, _ = load(block, peer, score)
            assert inputs == {job["job_key"]: job["payload_sha256"] for job in others}
            row[peer] = dict(raw_sha256=score["raw_sha256"], **compare(jobs, others))
        output[index] = row
    return output


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
