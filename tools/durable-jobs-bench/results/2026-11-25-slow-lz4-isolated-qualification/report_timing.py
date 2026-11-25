#!/usr/bin/env python3
"""Publish paired deltas from independently replayed codec timing blocks."""
import json
from pathlib import Path
import sys


CLOCKS = ("submit_to_last_settlement_ms", "commit_to_last_settlement_ms")
PEERS = ("river", "oban", "apalis")
POLICIES = ("pglz", "lz4")


def paired_blocks(rows):
    if set(rows) != {str(number) for number in range(1, 6)}:
        raise ValueError("all five original rotations are required")
    result = {}
    for group, arms in rows.items():
        result[group] = {}
        for policy in POLICIES:
            mesh = arms[f"measured-bulk-{policy}"]
            result[group][policy] = {
                peer: {clock: mesh[clock] - arms[f"measured-{peer}-{policy}"][clock] for clock in CLOCKS}
                for peer in PEERS
            }
    return result


def main():
    source = Path(sys.argv[1])
    replay = json.loads(source.read_text())
    if replay["series"] != "ordinary-slow-storage-transfer-isolated-pglz-v-lz4":
        raise ValueError("expected the isolated codec series")
    replay["within_block_mesh_minus_peer_ms"] = paired_blocks(replay["rows"])
    print(json.dumps(replay, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
