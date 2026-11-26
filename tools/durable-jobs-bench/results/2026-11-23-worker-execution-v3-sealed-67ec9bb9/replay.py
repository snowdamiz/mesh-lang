"""Verify and reduce the sealed native capture offline."""
import hashlib
import json
from pathlib import Path
import re
from statistics import median
import sys
import tarfile

ROOT = Path(__file__).resolve().parent
CAPTURE = ROOT / "capture"
SUITE = ROOT / "replay/frozen/tools/durable-jobs-bench"
sys.path.insert(0, str(SUITE))

import interleave
import run

PROOF_CASES = (
    "reuse/idle/fallback:", "timeout:", "stop during hold:", "stop during in-flight TLS write:",
    "stale/lost lease:", "cancel:", "crash:", "fail/retry:", "unsupported actor:",
    "independently expired its hard deadline", "ignored stale LIMIT/RENEW",
    "pre-entry expiry:", "second receipt dropped after its SQL commit and reset",
    "prior commits kept their exact XIDs", "Compiled Jobs parent death:",
)


def proof(pins, config):
    first, retry = (CAPTURE / "native" / name for name in ("first", "retry"))
    assert (first / "remote.exit").read_text().strip() == "1"
    assert (first / "archive.exit").read_text().strip() == "0"
    assert (retry / "proof.exit").read_text().strip() == "0"
    first_log = (first / "remote.log").read_text()
    assert "pg_ctl: could not start server" in first_log
    data_dir = re.search(r"creating directory (.+?)/data \.\.\. ok", first_log)[1]
    with tarfile.open(first / "setup-evidence.tar.gz") as archive:
        build = archive.extractfile("./mesh.json").read()
        assert build == (ROOT / "source/mesh.native.json").read_bytes()
        init_log = archive.extractfile("./fault-proof.log").read()
        assert b"waiting for server to start.... stopped waiting" in init_log
        for binary, expected in pins["proof_binaries"].items():
            assert hashlib.sha256(archive.extractfile(f"./compiled/{binary}").read()).hexdigest() == expected
    hashes = {}
    for line in (retry / "proof-driver.log").read_text().splitlines():
        checksum, path = line.split(maxsplit=1)
        hashes[Path(path).name] = checksum
    assert hashes == pins["proof_binaries"]
    with tarfile.open(retry / "proof.tar.gz") as archive:
        assert archive.extractfile("proof.exit").read().strip() == b"0"
        log = archive.extractfile("fault-proof.log").read().decode()
        assert all(case in log for case in PROOF_CASES)
        retry_dir = re.search(r"creating directory (.+?)/data \.\.\. ok", log)[1]
    smokes = {}
    for mode in ("strict", "trusted-persistent"):
        dest = retry / mode
        inventory = run.read_json(dest / "inventory.json")
        assert inventory == config["lanes"]["mesh-" + mode]["inventory"]
        folder = dest / "diagnostic-mesh"
        score = run.score_arm(folder, {"profile": "noop"}, "diagnostic", "mesh", inventory)
        with tarfile.open(folder / "raw.tar.gz") as archive:
            witness = run.file_json(archive, "adapter/mesh-execution.json")
        assert witness["mode"] == mode
        assert dict(score, execution=witness) == run.read_json(dest / "reconciled.json")
        assert score["completed"] == witness["jobs"] == 1024
        smokes[mode] = witness
    manifest = run.read_json(ROOT / "source/mesh.native.json")
    assert manifest["version"] == pins["candidate_revision"]
    assert "actual compiler source=" + pins["compiler_source_revision"] in first_log
    for mode in ("strict", "trusted-persistent"):
        mesh = config["lanes"]["mesh-" + mode]["inventory"]["engines"]["mesh"]
        assert mesh["artifact_sha256"] == hashlib.sha256(build).hexdigest()
        assert any(value == manifest["files"]["mesh"] for value in mesh["verified_files"].values())
    return {"first_exit": 1, "retry_exit": 0, "passed_cases": list(PROOF_CASES),
            "first_socket_path_bytes": len(f"{data_dir}/pgsocket/.s.PGSQL.55431".encode()),
            "retry_socket_path_bytes": len(f"{retry_dir}/pgsocket/.s.PGSQL.55431".encode()),
            "smokes": smokes}


def memory(archive, ledger, runners):
    raw = run.file_json(archive, "adapter/raw.json")
    samples = raw["samples"]
    peak = max(raw["idle_samples"] + samples, key=lambda s: s["rss_bytes"])
    drain = next(s for s in samples if s["terminal"] == len(ledger["jobs"]) and s["executing"] == 0)
    first = raw["idle_samples"][-1]
    assert peak["rss_bytes"] == ledger["peak_runner_rss_bytes"]
    for s in (first, peak, drain):
        assert s["root_rss_bytes"] + s["descendant_rss_bytes"] == s["rss_bytes"]
    fields = ("rss_bytes", "root_rss_bytes", "descendant_rss_bytes", "processes",
              "threads", "terminal", "executing")
    return {
        key: {**{field: s[field] for field in fields}, "descendants": s["processes"] - runners}
        for key, s in (("ready_idle", first), ("rss_peak", peak), ("drain", drain))
    }


def replay():
    pins = run.read_json(ROOT / "pins.json")
    for relative, info in pins["archived_original_inputs"].items():
        assert run.digest(ROOT / relative) == info["sha256"], relative
    for name, checksum in pins["matrix_drivers"].items():
        assert run.digest(SUITE / name) == checksum
    series = CAPTURE / "series"
    manifest = run.read_json(series / "series.json")
    config = manifest["config"]
    assert manifest["drivers"] == pins["matrix_drivers"] and config["cpus"] == [0, 1]
    assert run.read_json(CAPTURE / "native/retry/series-config.json") == config
    dispatch = run.read_json(series / "dispatch-1d579da98382.json")
    assert dispatch["exit_status"] == 0 and dispatch["groups"] == [1]
    assert dispatch["config_sha256"] == run.digest(CAPTURE / "native/retry/series-config.json")
    interleave.validate(config)
    built = proof(pins, config)
    rows = []
    for block in manifest["plan"]:
        if block["group"] not in dispatch["groups"]:
            continue
        attempt, receipt = interleave.select_attempt(series, config, block, {})
        path = series / "blocks" / attempt
        assert receipt == run.read_json(path / "reconciled.json")
        for phase, label, folder in interleave.arms(path, config, block):
            if phase != "measured":
                continue
            score = receipt["scores"][label]
            with tarfile.open(folder / "raw.tar.gz") as archive:
                ledger = run.file_json(archive, "adapter/ledger.json")
                jobs = ledger["jobs"]
                claims = [max(0, job["settled_us"] - job["claimed_us"]) / 1000
                          for job in jobs if "claimed_us" in job]
                row = {"profile": block["profile"], "series": label, "completed": score["completed"],
                       "independent_application_commits": 2 * score["ordered_independent_writes"],
                       "durable_jobs_per_s": score["commit_jobs_per_s"],
                       "runner_tree_cpu_ms_per_job_including_lifecycle": score["runner_cpu_ms_per_job"],
                       "pinned_cpu_ms_per_job_including_pg_and_driver_in_work_window":
                           score["pinned_user_cpu_ms_per_job"] + score["pinned_kernel_cpu_ms_per_job"],
                       "claim_to_terminal_p50_ms": median(claims) if len(claims) == len(jobs) else None,
                       "peak_rss_bytes": score["peak_runner_rss_bytes"], "peak_connections": score["peak_connections"]}
                if label.startswith("mesh-"):
                    witness = run.file_json(archive, "adapter/mesh-execution.json")
                    mode = label.removeprefix("mesh-")
                    distinct = len({job["worker_pid"] for job in jobs})
                    assert witness["mode"] == mode and witness["distinct_worker_pids"] == distinct
                    assert len({job["settled_xid"] for job in jobs}) == len(jobs)
                    if mode == "strict":
                        assert distinct == len(jobs)
                    else:
                        assert 0 < distinct <= ledger["slots"] and distinct < len(jobs)
                    row.update(distinct_worker_pids=distinct, memory=memory(archive, ledger, ledger["runners"]))
                rows.append(row)
    assert len(rows) == 10
    return {"measured_group": 1, "cpu_ids": config["cpus"], "native_lifecycle": built, "rows": rows}


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True, allow_nan=False))
