"""Price the last possible first commit for a 20% throughput lead at 512 jobs."""
import json
from pathlib import Path
import runpy
import sys
import tarfile
from hashlib import sha256

HERE = Path(__file__).resolve().parent
historic = runpy.run_path(str(HERE.parent / "2026-11-23-iteration-comparison/slow-peer-cutoffs.py"))


def price(directory):
    release = (directory / "controller-release.json").read_bytes()
    assert sha256(release).hexdigest() == "1bd34ec7973f01b2d57c7ba93e27741e03906f088d1d9962f99eedf09cdebde1"
    archives = json.loads(release)["raw_archives"]
    candidates = []
    for block in (1, 2):
        name = f"blocks/{block:02d}-slow/measured-combined/measured-mesh/raw.tar.gz"
        path = directory / name
        assert sha256(path.read_bytes()).hexdigest() == archives[name]
        with tarfile.open(path) as raw:
            ledger = json.load(raw.extractfile("adapter/ledger.json"))
        tx, = ledger["transactions"]
        assert ledger["input_sha256"] == historic["INPUT"]
        candidates.append((tx, ledger["jobs"], max(j["settled_us"] for j in ledger["jobs"])))

    def compare(mesh, peer):
        tx, jobs, finish = mesh
        other_tx, _, other_finish = peer
        assert all(j["hold_ms"] == 100 and j["effect_committed_us"] >= j["attempt_committed_us"] + 100_000
                   for j in jobs)
        assert len(jobs) == len({j["settled_xid"] for j in jobs}) == 512
        result = {}
        for label, clock in (("submit", "submitted_us"), ("drain", "committed_us")):
            target = (other_finish - other_tx[clock]) * 5 // 6
            first_due = tx[clock] + target - 100_000
            first = [j["attempt_committed_us"] for j in jobs]
            result[label] = dict(target_us=target, finish_minus_target_us=finish-tx[clock]-target,
                                 last_first_commit_minus_deadline_us=max(first)-first_due,
                                 first_commits_past_deadline=sum(v > first_due for v in first))
        result["enqueue_budget_at_observed_drain_us"] = result["submit"]["target_us"] - (finish-tx["committed_us"])
        return result

    output = []
    for i in range(1, 6):
        score_path, = historic["EVIDENCE"].glob(f"timing-*/{i:02d}-slow/reconciled.json")
        scores = json.loads(score_path.read_bytes())
        def load(lane):
            name = f"measured-{lane}-external"
            return historic["read_arm"](score_path.parent, name, scores[name])
        stock = load("bulk")
        for peer in ("river", "oban"):
            other = load(peer)
            output.append(dict(peer=peer, historical_block=i, old_external_mesh=compare(stock, other),
                               public_default_forward=compare(candidates[0], other),
                               public_default_reverse=compare(candidates[1], other)))
    return output


if __name__ == "__main__":
    print(json.dumps(price(Path(sys.argv[1])), indent=2, sort_keys=True))
