"""Reconcile the width proof and matched arms from retained raw ledgers."""
import argparse
import gzip
import json
from pathlib import Path
import re
import shutil
from statistics import median
import tarfile

from compare import ORDERS, screen, topology
import results as narrow

HERE = Path(__file__).resolve().parent
DEFINITIONS = {
    "width32": "b3667609e11055f0c121f083fa5121808d2ab6c865baa988da75a7d8edc47280",
    "width32_proof": "8989704216bc85cc379302e11bdb7af025beb59b0b8d73af1310de4548a2951e",
}
METRICS = (*narrow.KEYS, "first_submit_to_commit_ms", "admission_hints")


def replay(archive, orders, build):
    groups = []
    with tarfile.open(archive) as tar:
        def read(name):
            return json.load(tar.extractfile(name))

        run = read("series.json")
        assert run["input"] == topology.INPUT and run["layout"] == "tuned-2x32" and run["orders"] == orders
        assert run["compare_sha256"] == topology.sha(HERE / "compare.py") and run["built"] == build
        original = topology.read(screen.TOPOLOGY / "local-pins.json")
        assert run["stock"] == original
        for number, order in enumerate(orders, 1):
            group = {}
            for label in order:
                base = f"{number:02}/{label}"
                status = read(base + ".status.json")
                assert status["exit_code"] == 0
                def file(name):
                    return read(base + "/" + name)

                assert file("grant-width.json") == dict(label=label, stock_source=narrow.STOCK_SOURCE,
                        installed_source=DEFINITIONS.get(label, narrow.STOCK_SOURCE))
                pins = (dict(original, version="3828f20d2d6beb683868cce33e6dd1958365914b")
                        if label == "original" else dict(build, version=build["source"]["recipe_commit"]))
                score = narrow.baseline.verify_arm(file, lambda name: tar.extractfile(base + "/" + name).read().decode(),
                        "tuned-2x32", pins, local=True, sample_activity=False)
                # The stock verifier checks an extra, stricter cap-16 count inequality throughout.
                # Recompute the lower bound under the actual width for the published interval.
                jobs = file("ledger.json")["jobs"]
                cap = 32 if label in DEFINITIONS else 16
                min_calls = sum(narrow.baseline.minimum_turns(
                    [(j["claimed_us"], j["entered_us"]) for j in jobs if j["worker_pid"] == pid], cap)
                    for pid in file("topology.json")["runner_pids_by_index"])
                assert 0 < min_calls <= score["max_granting_calls"]
                group[label] = {k:score[k] for k in METRICS}
                group[label].update(lower_bound_grant_calls=min_calls, load_start=status["load_before"][0],
                                    unique_xids=score["unique_product_work_terminal_xids"],
                                    warm_app_sessions=score["warm_application_sessions"],
                                    max_jobs_per_runner=max(score["peak_slots_by_runner_index"]))
            groups.append(group)
    return groups


def seal(build, proof, measured):
    for source, destination in ((build / "built.json", "build.json"), (build / "artifacts/mesh.json", "manifest.json")):
        shutil.copyfile(source, HERE / destination)
    narrow.bundle(proof, HERE / "proof.tar.gz")
    narrow.bundle(measured, HERE / "measured.tar.gz")
    # Store original server lines for independent reduction, excluding unrelated SQL and startup secrets.
    text = (proof / "01/width32_proof/postgres.log").read_text()
    rows = [line for line in text.splitlines() if re.search(r'LOG:  widthproof count=\d+$', line)]
    assert rows
    (HERE / "proof-counts.log.gz").write_bytes(gzip.compress(("\n".join(rows) + "\n").encode(), mtime=0))


def main():
    arg = argparse.ArgumentParser(description=__doc__)
    arg.add_argument("--seal", nargs=3, type=Path, metavar=("BUILD", "PROOF", "MEASURED"))
    args = arg.parse_args()
    if args.seal:
        seal(*args.seal)
    build = topology.read(HERE / "build.json")
    assert topology.sha(HERE / "manifest.json") == build["manifest_sha256"]
    manifest = topology.read(HERE / "manifest.json")
    assert manifest["files"]["mesh"] == build["executable_sha256"]
    assert manifest["compiler_sha256"] == build["compiler_sha256"]
    assert manifest["experiment"] == build["source"] and manifest["runtime_sha256"] == build["runtime_sha256"]
    proof = replay(HERE / "proof.tar.gz", [["width32_proof"]], build)
    counts = [int(re.search(r' count=(\d+)$', ln)[1]) for ln in
              gzip.decompress((HERE / "proof-counts.log.gz").read_bytes()).decode().splitlines()]
    assert sum(counts) == 1024 and max(counts) == 32
    groups = replay(HERE / "measured.tar.gz", ORDERS, build)
    medians = {name:{k:median(group[name][k] for group in groups) for k in METRICS}
               for name in ORDERS[0]}
    better = {}
    for field, lower in (("submit_jobs_per_s", False), ("commit_jobs_per_s", False),
                         ("runner_cpu_ms_per_job", True)):
        both, spread, paired = 0, 0, []
        for group in groups:
            a, b = [group[k][field] for k in ("width16_a", "width16_b")]
            value = group["width32"][field]
            margin = min(a,b) - value if lower else value - max(a,b)
            both += margin > 0
            spread += margin > abs(a-b)
            paired.append(100 * (value / median((a,b)) - 1))
        better[field] = dict(beats_both=both, beyond_control_spread=spread, paired_change_pct=paired,
                             paired_median_pct=median(paired))
    out = dict(groups=groups, lane_medians=medians, width32_comparison=better, proof=proof[0],
               proof_over_16=sum(n>16 for n in counts), proof_max=max(counts),
               archives={name:topology.sha(HERE/name) for name in ("proof.tar.gz", "measured.tar.gz", "proof-counts.log.gz")})
    encoded = json.dumps(out, indent=2, sort_keys=True) + "\n"
    if args.seal:
        (HERE / "result.json").write_text(encoded)
    else:
        assert (HERE / "result.json").read_text() == encoded
    print("Verified the 1,024-job proof and 16,384 measured jobs.")
    print(json.dumps(better))


if __name__ == "__main__":
    main()
