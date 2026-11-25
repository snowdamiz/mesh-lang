#!/usr/bin/env python3
"""Continue at the first unfinished whole rotation under a new controller grant."""
import argparse
import fcntl
import json
from pathlib import Path
import shlex

import recover

recover.bind_recovery()
packet = recover.packet
suite = packet.matrix
BASE = packet.OUT / "timing"
OUT = packet.OUT / "timing-continuation"
REQUEST = packet.HERE / "timing-continuation-packet.json"
FIRST_GROUPS = [1, 2]
NEW_GROUPS = [3, 4, 5]
ENGINES = ("bulk", "river", "oban", "apalis")
POLICIES = ("pglz", "lz4")
METRICS = (*packet.CLOCKS, "first_submit_to_commit_ms", "producer_ack_ms", "producer_sql_exec_ms",
           "commit_to_first_application_commit_ms", "commit_to_64th_first_application_commit_ms",
           "commit_to_final_first_application_commit_ms", "64th_to_final_first_application_commit_ms")


def released(id_, seal):
    path = packet.QUEUE / f"{id_}.released.json"
    receipt = packet.read(path)
    suite.require(receipt["id"] == id_ and receipt["released_at"]
                  and receipt["request_sha256"] == seal, "controller release does not match the sealed request")
    return path, receipt


def raw_pins(directory):
    return {str(path.relative_to(directory)): packet.digest(path)
            for path in sorted(directory.glob("*-slow/*/*/raw.tar.gz"))}


def retained(original, config):
    original_seal = packet.digest(packet.TIMING)
    released_path, receipt = released(original["id"], original_seal)
    suite.require(receipt["exit_status"] != 0
                  and packet.read(BASE / "grant.json")["packet_sha256"] == original_seal,
                  "expected an interrupted original grant")
    rows = {
        str(group): packet.replay_block(BASE / f"{group:02d}-slow", group, config, original["phases"])
        for group in FIRST_GROUPS
    }
    suite.require(sorted(p.name for p in BASE.glob("*-slow")) == ["01-slow", "02-slow", "03-slow"]
                  and not (BASE / "03-slow/reconciled.json").exists(),
                  "original interruption is not confined to rotation 3")
    interruption_path = BASE / "interruption.json"
    interruption = packet.read(interruption_path)
    raw = raw_pins(BASE)
    suite.require(raw == receipt["raw_sha256"] and receipt["completed_groups"] == FIRST_GROUPS
                  and receipt["interrupted_group"] == 3
                  and interruption["arm"] == "03-slow/measured-oban-pglz"
                  and interruption["first_undispatched"] == "03-slow/measured-apalis-lz4"
                  and interruption["observed_remote_status"]["exit_code"] == 0
                  and interruption["recovered_raw_sha256"] ==
                  raw["03-slow/measured-oban-pglz/measured-oban/raw.tar.gz"],
                  "original exit, first undispatched arm, or raw custody differs")
    captured = {}
    rotation = BASE / "03-slow"
    order = config["orders"][2]
    suite.require(packet.read(rotation / "order.json") == dict(group=3, order=order, phases=original["phases"]),
                  "partial rotation changed its original order")
    for phase in original["phases"]:
        for label in order:
            lane = config["lanes"][label]
            arm = rotation / f"{phase}-{label}"
            evidence = arm / f"{phase}-{lane['engine']}/raw.tar.gz"
            if evidence.is_file():
                captured[f"{phase}-{label}"] = packet.score(arm, label, config, phase)
    suite.require(set(captured) == {f"warmup-{label}" for label in order} | {f"measured-{order[0]}"}
                  and len(raw) == 41, "original capture set or recovered Oban receipt differs")
    return rows, captured, {
        "original_packet_sha256": original_seal,
        "original_release_sha256": packet.digest(released_path),
        "original_raw_sha256": raw,
        "original_interruption_sha256": packet.digest(interruption_path),
        "retained_groups": FIRST_GROUPS,
        "excluded_original_groups": [3],
    }


def expected_packet(original, config, custody):
    command = f"python3 -B {shlex.quote(str(Path(__file__).resolve()))}"
    groups = NEW_GROUPS
    phases = original["phases"]
    return dict(
        custody,
        id=original["id"] + "-continue-3-5", kind="TIMING", requested_window_minutes=30,
        setup_release_sha256=original["setup_release_sha256"],
        inventory=original["inventory"], inventory_sha256=original["inventory_sha256"],
        controller_source_sha256=packet.digest(Path(__file__)),
        build=False, groups=groups, phases=phases, arms=len(groups) * len(phases) * len(config["lanes"]),
        orders={str(group): config["orders"][group - 1] for group in groups},
        evidence_destination=str(OUT), offline_check_command=command + " check",
        timing_command=command + " timing", replay_command=command + " replay",
    )


def dispatch(plan, config):
    packet.grant("TIMING", REQUEST, 300)
    with (packet.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        admission = packet.grant("TIMING", REQUEST, 300)
        OUT.mkdir()
        packet.write(OUT / "grant.json", admission)
        suite.ssh(["bash", "-c", packet.original.HOST_IDLE], timeout=12).check_returncode()
        packet.dispatch(OUT, config, [(group, plan["phases"]) for group in plan["groups"]], REQUEST, "TIMING")
        suite.ssh(["bash", "-c", packet.original.HOST_IDLE], timeout=12).check_returncode()


def timing_deltas(rows):
    within_engine, versus_peer = {}, {}
    for group, arms in rows.items():
        within_engine[group] = {}
        for engine in ENGINES:
            pglz, lz4 = (arms[f"measured-{engine}-{policy}"] for policy in POLICIES)
            within_engine[group][engine] = {
                key: pglz[key] - lz4[key] for key in METRICS if key in pglz and key in lz4
            }
        versus_peer[group] = {
            policy: {engine: {key: arms[f"measured-bulk-{policy}"][key] - arms[f"measured-{engine}-{policy}"][key]
                              for key in packet.CLOCKS} for engine in ENGINES[1:]} for policy in POLICIES
        }
    summarize = packet.original.distribution
    codec = {
        engine: {key: summarize([block[engine][key] for block in within_engine.values()])
                 for key in next(iter(within_engine.values()))[engine]
                 if all(key in block[engine] for block in within_engine.values())}
        for engine in ENGINES
    }
    peers = {
        policy: {engine: {key: summarize([block[policy][engine][key] for block in versus_peer.values()])
                          for key in packet.CLOCKS} for engine in ENGINES[1:]} for policy in POLICIES
    }
    return dict(paired_pglz_minus_lz4=codec, within_block_pglz_minus_lz4=within_engine,
                within_block_mesh_minus_peer=peers, within_block_mesh_minus_peer_ms=versus_peer)


def replay(plan, config, rows, excluded):
    release_path, release = released(plan["id"], packet.digest(REQUEST))
    suite.require(packet.read(OUT / "grant.json")["packet_sha256"] == packet.digest(REQUEST),
                  "continuation grant did not bind the request")
    for group in plan["groups"]:
        block = OUT / f"{group:02d}-slow"
        if (block / "reconciled.json").is_file():
            rows[str(group)] = packet.replay_block(block, group, config, plan["phases"])
    raw = raw_pins(OUT)
    if "raw_sha256" in release:
        suite.require(raw == release["raw_sha256"], "continuation raw archives differ from controller custody")
    selected = [arm for group in rows.values() for arm in group.values()]
    all_attempts = [arm["attempt_id"] for arm in (*selected, *excluded.values())]
    suite.require(len(set(all_attempts)) == len(all_attempts), "an attempt was reused across rotations")
    complete = release["exit_status"] == 0 and len(rows) == 5 and len(selected) == 80
    suite.require(release["exit_status"] != 0 or complete, "successful release lacks the complete capture set")
    return dict(timing_deltas(rows), series="ordinary-slow-storage-transfer-isolated-pglz-v-lz4",
                full_comparison_eligible=complete, completed_groups=[int(g) for g in rows],
                selected_arm_count=len(selected), original_partial_arms=len(excluded), rows=rows,
                original_release_sha256=plan["original_release_sha256"],
                continuation_release_sha256=packet.digest(release_path),
                continuation_raw_sha256=raw)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("seal", "check", "timing", "replay"))
    action = parser.parse_args().action
    setup, source = packet.check()
    original, config = packet.check_timing(setup, source)
    rows, excluded, custody = retained(original, config)
    plan = expected_packet(original, config, custody)
    if action == "seal":
        packet.write(REQUEST, plan)
    else:
        suite.require(packet.read(REQUEST) == plan, "continuation seal or retained native custody changed")
        if action == "check":
            print(packet.digest(REQUEST))
        elif action == "timing":
            dispatch(plan, config)
        else:
            print(json.dumps(replay(plan, config, rows, excluded), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
