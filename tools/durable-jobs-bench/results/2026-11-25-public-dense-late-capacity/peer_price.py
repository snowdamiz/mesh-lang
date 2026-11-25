"""Read the released matched-peer clocks and compute required Mesh savings."""
import json
from hashlib import sha256

from replay import ROOT, fetch

REVISION = "893710708"
EVIDENCE = f"{ROOT}/path-recovery/peer-stage-recovery"
REVERSE = f"{EVIDENCE}/reverse-continuation/evidence/peers"
PEERS = dict(river="go", oban="oban", apalis="rust")


def read(path):
    return json.loads(fetch(REVISION, path))


def price():
    proof = read(f"{REVERSE}/independent-replay.json")
    release = read(f"{REVERSE}/controller-release.json")
    assert release["exit_status"] == 0 and release["id"] == proof["reverse_source"]
    groups = {}
    for block in (1, 2):
        rows = proof["groups"][str(block)]
        source = f"{EVIDENCE}/evidence/peers" if block == 1 else REVERSE
        for label, engine in dict(B="mesh", A1="mesh", A2="mesh", **PEERS).items():
            raw = fetch(REVISION, f"{source}/blocks/{block:02}-noop/measured-{label}/measured-{engine}/raw.tar.gz")
            assert sha256(raw).hexdigest() == rows["measured-" + label]["raw_sha256"]
        mesh = rows["measured-B"]
        groups[block] = {}
        for peer in PEERS:
            other = rows["measured-" + peer]
            result = {}
            for clock in ("submit_us", "drain_us"):
                spread = abs(rows["measured-A1"][clock] - rows["measured-A2"][clock])
                gap = mesh[clock] - other[clock]
                margin = max(0.05 * other[clock], spread)
                result[clock] = dict(mesh_minus_peer_us=gap, historical_margin_us=margin,
                                     savings_to_pass_historical_margin_us=max(0, gap + margin))
            groups[block][peer] = result
    return dict(source=REVISION, release=release["id"], rotations=groups)


if __name__ == "__main__":
    print(json.dumps(price(), indent=2, sort_keys=True))
