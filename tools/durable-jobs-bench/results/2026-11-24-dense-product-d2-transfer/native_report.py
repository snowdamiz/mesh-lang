"""Replay the six public dense rotations from their sealed native transaction ledgers."""
from hashlib import sha256
import json
from pathlib import Path
from statistics import median
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = HERE / "evidence/native-closure-retry"
LANES = ["hot", "checked-a", "checked-b", "oban-2x32", "river-2x32", "apalis-2x32"]
NAMES = ["Mesh hot", "Mesh checked A", "Mesh checked B", "Oban", "River", "Apalis"]


def check(condition, message):
    if not condition:
        raise ValueError(message)


def replay():
    summary = json.loads((ROOT / "final-replay.json").read_bytes())
    blocks = summary["blocks"]
    check(len(blocks) == 6 and len(summary["raw"]) == 72 and not summary["missing"], "incomplete series")
    measured = []
    for group in blocks.values():
        arms = {}
        for label in LANES:
            for phase in ("warmup", "measured"):
                arm = group["scores"][f"{phase}-{label}"]
                folder = ROOT / "blocks" / group["attempt"] / f"{phase}-{label}"
                path, = folder.glob("*/raw.tar.gz")
                check(sha256(path.read_bytes()).hexdigest() == summary["raw"][str(path.relative_to(ROOT))]
                      == arm["raw_sha256"], f"{path} digest differs")
                with tarfile.open(path) as archive:
                    ledger = json.load(archive.extractfile("adapter/ledger.json"))
                jobs = ledger["jobs"]
                check(len(jobs) == arm["completed"] == 1024 and arm["failed"] == 0, "wrong completed work")
                check(len({int(j[key]) for key in ("attempt_xid", "effect_xid") for j in jobs}) == 2048,
                      "missing or repeated application transaction")
                txn, = ledger["transactions"]
                start = txn["committed_us"]
                settle = max(j["settled_us"] for j in jobs)
                submit = (settle - txn["submitted_us"]) / 1000
                drain = (settle - start) / 1000
                check(abs(1024000 / submit - arm["submit_jobs_per_s"]) < 0.00001 and
                      abs(1024000 / drain - arm["commit_jobs_per_s"]) < 0.00001, "clock differs")
                writes = sorted((j["effect_committed_us"] - start) / 1000 for j in jobs)
                entries = sorted((j["entered_us"] - start) / 1000 for j in jobs)
                if phase == "measured":
                    arms[label] = dict(submit=submit, drain=drain, writes=writes, entries=entries,
                                       terminal=len({j["settled_xid"] for j in jobs}))
        measured.append(arms)
    return measured, summary


def report():
    measured, summary = replay()
    lines = ["# Public dense queue with hot PostgreSQL checkout", "",
             "Six Linode rotations used two CPUs shared with PostgreSQL 17.11. Every engine used two "
             "32-slot runners. The three Mesh lanes used the same public trusted executable and grouped "
             "successful completions after actor teardown. Every measured arm and warmup completed "
             "1,024 jobs and reconciled 2,048 distinct application transactions.", "",
             "| Lane | Submit to settlement, jobs/s | Commit to settlement, jobs/s | Hot wins, submit / drain |",
             "|---|---:|---:|---:|"]
    for label, name in zip(LANES, NAMES):
        rates = [median(1024000 / r[label][k] for r in measured) for k in ("submit", "drain")]
        wins = [sum(r["hot"][k] < r[label][k] for r in measured) for k in ("submit", "drain")]
        pairs = "—" if label == "hot" else f"{wins[0]}/6 · {wins[1]}/6"
        lines.append(f"| {name} | {rates[0]:,.2f} | {rates[1]:,.2f} | {pairs} |")
    winners = ", ".join(map(str, summary["qualifying_groups"])) or "none"
    verdict = "passed" if summary["advances"] else "missed"
    probes = {label: sorted({g["scores"][f"measured-{label}"]["checkout_probes"] for g in summary["blocks"].values()})
              for label in LANES[:3]}
    def counts(observed):
        return str(min(observed)) if min(observed) == max(observed) else f"{min(observed)}–{max(observed)}"

    lines.extend(["", f"The hot pool {verdict} the predeclared four-of-six advancement rule. "
                  f"Qualifying groups were {winners}. Hot checkout made {counts(probes['hot'])} probes per block. "
                  f"The checked lanes made {counts(probes['checked-a'] + probes['checked-b'])} probes per block.", "",
                  "## Work remaining when a peer finishes", "",
                  "Each comparison aligns its own enqueue commit to zero. Counts show how many Mesh jobs "
                  "still had to enter their handler or durably commit their second write when the paired peer "
                  "had settled its last job. Values are medians of six within-rotation differences or counts.", "",
                  "| Peer | Extra Mesh drain, ms | Jobs still to enter | Second writes still to commit | "
                  "Extra Mesh time for second writes 103–922, ms |",
                  "|---|---:|---:|---:|---:|"])
    for label, name in zip(LANES[3:], NAMES[3:]):
        rows = []
        for arms in measured:
            hot, peer = arms["hot"], arms[label]
            cutoff = peer["drain"]
            rows.append([hot["drain"] - cutoff, sum(t > cutoff for t in hot["entries"]),
                         sum(t > cutoff for t in hot["writes"]),
                         (hot["writes"][921] - hot["writes"][102]) - (peer["writes"][921] - peer["writes"][102])])
        values = [median(col) for col in zip(*rows)]
        lines.append(f"| {name} | {values[0]:+.1f} | {values[1]:g} | {values[2]:g} | {values[3]:+.1f} |")
    tails = [r["hot"]["drain"] - r["hot"]["writes"][-1] for r in measured]
    lines.extend(["", f"Mesh's last application commit preceded final settlement by {min(tails):.1f}–{max(tails):.1f} ms. "
                  "Earlier settlement can also change when slots refill. The ledgers measure the overlap "
                  "and leave the CPU and lock costs of each stage open.", "",
                  "Run `python3 -B native_report.py` to reproduce this report from the archived ledgers. "
                  "The original controller decision and all measured and warmup archive hashes are in "
                  "[final-replay.json](evidence/native-closure-retry/final-replay.json).", ""])
    return "\n".join(lines)


if __name__ == "__main__":
    print(report(), end="")
