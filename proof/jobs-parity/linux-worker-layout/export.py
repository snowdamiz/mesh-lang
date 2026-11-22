"""Export compact replay inputs from the retained local qualification archive."""

import argparse
import csv
from hashlib import sha256
import json
from pathlib import Path
import shlex

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
DEFAULT_ARCHIVE = ROOT / "target/worker-hot-section-layout"
FROZEN_HARNESS_SHA = "03252978489916ff37f7c42712bd6dbe9d96254864d8f4d2060c4b39175d6c24"
ARMS = ("mesh-stock", "mesh-candidate", "oban")


def load(path):
    return json.loads(path.read_text())


def digest(path):
    with path.open("rb") as stream:
        return sha256(stream.read()).hexdigest()


def dump(name, data):
    (HERE / name).write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")


def link_commands(archive):
    # The retained link commands use absolute paths from the build checkout.
    # Remove its prefix and keep only paths relative to that checkout.
    commands = {}
    inputs = {}
    for variant in ("stock", "exact-inputs"):
        link = archive / variant
        actual = shlex.split((link / "mesh.link.txt").read_text())
        checkout_prefix, marker, _ = actual[0].partition("/target/")
        assert checkout_prefix.startswith("/") and marker == "/target/"
        prefix = checkout_prefix + "/"

        def relative(value):
            return value.replace(prefix, "")

        commands[variant] = [relative(value) for value in actual]
        inputs.update({relative(path): checksum for path, checksum in load(link / "inputs.json").items()})
    assert all(not value.startswith("/") or value == "/lib/ld-linux-aarch64.so.1"
               for argv in commands.values() for value in argv)
    assert all(not path.startswith("/") and ".." not in Path(path).parts for path in inputs)
    return commands, inputs


def main(archive):
    score = load(archive / "qualification-score.json")
    settings = ROOT / "scripts/jobs-parity-profiles.json"
    harness = ROOT / "scripts/jobs-parity.py"
    assert digest(settings) == score["settings_sha256"]
    assert digest(harness) == FROZEN_HARNESS_SHA
    (HERE / "frozen-profiles.json").write_bytes(settings.read_bytes())
    archive_hint = DEFAULT_ARCHIVE.relative_to(ROOT).as_posix()
    batches = []
    intervals = []
    for group in score["groups"]:
        name = group["name"]
        out = archive / "qualification" / name
        evidence = out / "evidence"
        config = load(evidence / "config.json")
        receipt = load(out / "receipt.json")
        assert config["phase_series"] == "qualification"
        assert receipt["stock_mesh_sha256"] == group["stock_sha256"]
        assert receipt["mesh_elf_sha256"] == score["artifact_sha256"] == group["candidate_sha256"]
        arms = {}
        for arm in ARMS:
            recorded = group["measured"][arm]
            raw = evidence / arm / "raw.json"
            shutdown = evidence / arm / "shutdown.json"
            ledger = evidence / arm / "ordered-writes.json"
            attempts = load(raw)["attempts"]
            assert len(attempts) == recorded["ordered_write_pairs"] == 512
            assert len({row["job_key"] for row in attempts}) == 512
            assert all(row["attempt"] == row["attempts"] == row["effects"] == 1
                       and row["state"] == "completed" for row in attempts)
            origin_ms = int(min(row["recorded_ms"] for row in attempts))
            for row in sorted(attempts, key=lambda item: item["job_key"]):
                start = round((row["recorded_ms"] - origin_ms) * 1000)
                finish = round((row["effected_ms"] - origin_ms) * 1000)
                intervals.append((name, arm, start, finish))
            warmup = group["warmup"][arm]
            arms[arm] = {
                "metrics": recorded["measurements"],
                "ordered_writes": load(ledger),
                "observed_committed_hold_peak": recorded["actual_overlapping_committed_holds"],
                "shutdown": load(shutdown),
                "drained_mesh_status": [{key: status[key] for key in
                    ("claimed", "finished", "drained", "pending_claims", "undrained", "stale_acks", "last_error")}
                    for status in recorded["drained_mesh_status"]],
                "warmup_completed": warmup["measurements"]["completed"],
                "warmup_ordered_pairs": warmup["ordered_write_pairs"],
                "archive_sha256": {file.name: digest(file) for file in (raw, shutdown, ledger)},
            }
        batches.append({"name": name, "order": group["order"], "arms": arms,
                        "archive_path": f"{archive_hint}/qualification/{name}",
                        "manifest_sha256": digest(evidence / "config.json"),
                        "receipt_sha256": digest(out / "receipt.json")})
    with (HERE / "hold-intervals.csv").open("w", newline="") as stream:
        writer = csv.writer(stream, lineterminator="\n")
        writer.writerow(("group", "arm", "first_commit_us", "second_commit_us"))
        writer.writerows(intervals)

    held = archive / "canonical-held-01/evidence/candidate-trace"
    life = load(held / "lifecycle.json")
    markers = load(held / "held.json")["workers"]
    xids = [row["pid"] for row in load(held / "commit-xids.json")]
    assert set(markers) == set(xids) and len(markers) == len(xids) == 64
    assert digest(held / "lifecycle.json") == score["held_exact_reap_scope"]["sha256"]
    first = score["groups"][0]
    example_receipt = load(archive / "qualification/slow-01/receipt.json")
    dump("qualification.json", {
        "schema": 1, "profile": "slow", "runners": 1, "slots": 64, "jobs_per_arm": 512,
        "candidate_sha256": score["artifact_sha256"], "stock_sha256": first["stock_sha256"],
        "frozen_source": {"revision": "03b20f42c253701641d591a710a0a13211f623d4",
                          "jobs-parity.py": FROZEN_HARNESS_SHA, "profiles": score["settings_sha256"]},
        "runtime": {"postgresql": "17.11", "oban_revision": example_receipt["oban_revision"],
                    "oban_erl_flags": example_receipt["oban_erl_flags"],
                    "oban_erl_inetrc_sha256": first["shared_component_sha256"]["proof/oban.inetrc"],
                    "load_geometry": example_receipt["fault_window_and_loader"],
                    "guest_base_sha256": first["guest_base_sha256"], "kernel_sha256": first["kernel_sha256"],
                    "launcher_sha256": first["launcher_sha256"], "worker_thp_enabled": 0},
        "shared_component_sha256": first["shared_component_sha256"],
        "hold_intervals_sha256": digest(HERE / "hold-intervals.csv"),
        "held_exact_reap": {
            "archive_path": f"{archive_hint}/canonical-held-01/evidence/candidate-trace",
            "candidate_sha256": score["artifact_sha256"],
            "held_pids": sorted(markers), "committed_pids": sorted(xids), **life,
            "archived_lifecycle_sha256": digest(held / "lifecycle.json"),
        },
        "recorded_result": {
            "frozen": score["frozen"]["passed"],
            "shutdown_inclusive": score["supplemental"]["shutdown_inclusive"]["passed"],
            "matched_full_occupancy": score["supplemental"]["matched_full_occupancy"]["passed"],
            "throughput_pair_wins": score["frozen"]["throughput_pair_wins"],
        },
        "groups": batches,
    })
    commands, inputs = link_commands(archive)
    dump("link-provenance.json", {
        "working_directory": "repository root",
        "release_link_argv": commands,
        "retained_link_input_sha256": inputs,
        "source_archive_hint": archive_hint,
        "portable_compiler_experiment_hint": "target/portable-mesh-section-layout/REPLAY.md",
        "input_and_proof_artifacts_sha256": {
            file: digest(archive / file) for file in
            ("link.py", "profile.py", "model.py", "audit.py", "REPLAY.md", "stock/mesh",
             "stock/link.map", "exact-inputs/order.ld", "exact-inputs/mesh",
             "exact-inputs/link.map", "exact-inputs/abi.json", "profiles.json",
             "training-model.json", "heldout-model.json", "held-summary.json",
             "pilot-score.json", "qualification-score.json", "rss-sample-analysis.json")
        },
    })


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--archive", type=Path, default=DEFAULT_ARCHIVE)
    opts = cli.parse_args()
    main(opts.archive.resolve())
