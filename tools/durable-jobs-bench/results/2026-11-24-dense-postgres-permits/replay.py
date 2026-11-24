"""Check all captured ledgers and reproduce the two dense permit contrasts."""
from hashlib import sha256
import json
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent


def clocks(row):
    return dict(submit=(row["last_settlement_us"]-row["first_submit_us"])/1000,
                drain=(row["last_settlement_us"]-row["first_commit_us"])/1000,
                middle=row["middle_ms"], final_write=(row["last_effect_us"]-row["first_commit_us"])/1000)


def main():
    results = {}
    for stage in ("diagnostic", "screen"):
        blocks = sorted((HERE / stage).glob("*/reconciled.json"))
        assert len(blocks) == (1 if stage == "diagnostic" else 2)
        for block in blocks:
            scores = json.loads(block.read_bytes())
            for label,row in scores.items():
                phase = label.split("-",1)[0]
                archive = block.parent / label / f"{phase}-mesh/raw.tar.gz"
                assert sha256(archive.read_bytes()).hexdigest() == row["raw_sha256"]
                with tarfile.open(archive) as raw:
                    ledger = json.load(raw.extractfile("adapter/ledger.json"))
                jobs = ledger["jobs"]
                assert len(jobs) == row["completed"] == 1024 and row["failed"] == 0
                assert all(j["attempts"] == j["effects"] == j["attempt"] == 1 and
                           j["attempt_committed_us"] < j["effect_committed_us"] <= j["settled_us"] for j in jobs)
                assert len({j[k] for j in jobs for k in ("attempt_xid","effect_xid")}) == 2048
                assert row["first_commit_us"] == ledger["transactions"][0]["committed_us"]
                assert row["last_settlement_us"] == max(j["settled_us"] for j in jobs)
            if stage == "diagnostic":
                continue
            a,b = (scores["measured-sync-"+s] for s in ("a","b"))
            decision = {}
            for mode in ("ready2","ready4"):
                c = scores["measured-"+mode]
                checks = {}
                for key, elapsed in clocks(c).items():
                    left,right = clocks(a)[key],clocks(b)[key]
                    threshold = 0 if key == "final_write" else max(abs(left-right),0.05*min(left,right))
                    checks[key] = min(left,right)-elapsed > threshold
                for key,factor in (("runner_cpu_ms_per_job",1.05),("peak_runner_rss_bytes",1.1)):
                    checks[key] = c[key] <= factor*max(a[key],b[key])
                def host(row):
                    return row["pinned_user_cpu_ms_per_job"]+row["pinned_kernel_cpu_ms_per_job"]
                checks["host_cpu"] = host(c) <= 1.05*max(host(a),host(b))
                decision[mode] = dict(passes=all(checks.values()), checks=checks)
            results[block.parent.name] = decision
    print(json.dumps(results,indent=2,sort_keys=True))


if __name__ == "__main__":
    main()
