"""Price the public SLOW checkout option from retained native archives."""

from collections import Counter
import hashlib
import json
from pathlib import Path
from statistics import median
import subprocess
import sys
import tarfile
from types import FunctionType

HERE = Path(__file__).resolve().parent
RESULTS = HERE.parent
ROOT = HERE.parents[3]
PUBLIC = RESULTS / "2026-11-24-trusted-jobs-product-composition"
DENSE = RESULTS / "2026-11-24-dense-product-d2-transfer"
SLOW = RESULTS / "2026-11-25-public-slow-file-topology"
RECOVERY = SLOW / "roster-retry"
INBOX = ROOT / "target/linode-jobs-stepchange/requests"
sys.path.insert(0, str(SLOW))
import history
from score import ELF, RUNNER, run, verify

REVISION = "b3557273d72be3802bf189b284e7caaf5f3caa0f"
MANIFEST = "979cd601099c5e7da4be763229a91e9dcabca51a35f4bed79f6cf4128adbac55"
SLOTS = 64
HOLD_US = 100_000
JOBS = 512


def json_member(archive, name):
    return run.file_json(archive, f"adapter/{name}.json")


def distribution(numbers):
    return dict(min=min(numbers), median=median(numbers), max=max(numbers))


def app_sql(archive, jobs):
    statements = json_member(archive, "trusted-application-statements")
    probe, = [row for row in statements if row["query"] == "SELECT $1"]
    assert probe["calls"] == probe["rows"]
    for name in ("attempts", "effects"):
        insert, = [r for r in statements if r["query"].startswith("INSERT INTO public.mesh_jobs_parity_" + name)]
        assert insert["calls"] == insert["rows"] == jobs
    return dict(calls=probe["calls"], execution_ms=probe["execution_ms"])


def image(archive, runners):
    build = json_member(archive, "build")
    assert build["files"]["mesh"] == ELF and build["version"] == REVISION
    ledger = json_member(archive, "ledger")
    assert ledger["artifact_sha256"] == MANIFEST and ledger["version"] == REVISION
    assert ledger["runners"] == runners and ledger["slots"] == SLOTS
    processes = json_member(archive, "process-identity")
    assert len(processes) == runners + 1
    assert all(p["argv"] == [RUNNER, "run"] and p["executable_sha256"] == ELF for p in processes[1:])
    assert all(p["cpus"] == [0, 1] for p in processes)
    return ledger


def dense_result():
    native = DENSE / "evidence/native-closure-retry"
    report = run.read_json(native / "final-replay.json")
    assert report["qualified_rotations"] == 6 and report["advances"] is True and not report["missing"]
    config = run.read_json(native / "series.json")["config"]
    assert config["profiles"] == ["noop"] and config["cpus"] == [0, 1]
    proof = config["product"]
    assert proof["build"]["files"]["mesh"] == ELF and proof["manifest_sha256"] == MANIFEST
    releases = sorted((HERE / "evidence/releases").glob("*.released.json"))
    released = []
    for path in releases:
        data = run.read_json(path)
        assert data["exit_status"] == 0 and data["id"] == "2026-11-24-dense-product-d2-transfer-closure-retry-grant-recovery"
        if (INBOX / path.name).is_file():
            assert run.digest(path) == run.digest(INBOX / path.name)
        released.extend(data["groups"])
    assert released == list(range(1, 7))
    phases = ("warmup", "measured")
    scope = dict(vars(verify), PROFILES={**verify.PROFILES, "noop": (2, *verify.PROFILES["noop"][1:])})
    reconcile = FunctionType(verify.reconcile.__code__, scope)
    blocks = []
    for index, block in sorted(report["blocks"].items()):
        measured = {}
        for phase in phases:
            for arm in ("checked-a", "checked-b", "hot"):
                key = f"{phase}-{arm}"
                relative = f"blocks/{block['attempt']}/{key}/{phase}-mesh/raw.tar.gz"
                recorded = block["scores"][key]
                path = native / relative
                assert run.digest(path) == report["raw"][relative] == recorded["raw_sha256"]
                with tarfile.open(path) as raw:
                    ledger = image(raw, 2)
                    assert ledger["phase"] == phase and ledger["queue_policy"] == "success_cohort"
                    assert run.file_json(raw, "status.json")["exit_code"] == 0
                    calls = app_sql(raw, 1024)
                    assert calls["calls"] == recorded["checkout_probes"] == (4 if arm == "hot" else 2048)
                    assert calls["execution_ms"] == recorded["probe_sql_exec_ms"]
                    native_tls = Counter(row["role"] for row in json_member(raw, "postgres-tls-sessions") if row["tls"])
                    assert native_tls["compiled_caller"] == 32 and native_tls["parity_producer"] == 1
                    assert 4 <= native_tls["parity_subject"] <= 8 and len(native_tls) == 3
                    score = reconcile(ledger, "mesh", "noop", dict(artifact_sha256=MANIFEST, version=REVISION))
                    edges = ("first_submit_us", "first_commit_us", "last_settlement_us")
                    assert score["completed"] == 1024 and all(score[k] == recorded[k] for k in edges)
                    assert max(j["effect_committed_us"] for j in ledger["jobs"]) == recorded["last_effect_us"]
                if phase == "measured":
                    measured[arm] = dict(
                        submit_us=score["last_settlement_us"] - score["first_submit_us"],
                        drain_us=score["last_settlement_us"] - score["first_commit_us"],
                        last_effect_us=recorded["last_effect_us"] - score["first_commit_us"],
                        probe=calls, ready_tls_by_role=dict(native_tls),
                        runner_lifetime_cpu_ms=recorded["runner_cpu_ms_per_job"] * 1024,
                    )
        hot = measured["hot"]
        a, b = (measured[name] for name in ("checked-a", "checked-b"))
        savings = {field: min(a[field], b[field]) - hot[field] for field in ("submit_us", "drain_us", "last_effect_us")}
        skipped = a["probe"]["calls"] - hot["probe"]["calls"]
        assert skipped == 2044 and all(savings[key + "_us"] ==
                                       row["faster_control_elapsed_us"] - row["hot_elapsed_us"]
                                       for key, row in block["contrast"]["clocks"].items())
        blocks.append(dict(group=int(index), measured=measured, savings_us=savings, skipped_probes=skipped,
                           wall_us_per_skipped_probe={key: value / skipped for key, value in savings.items()},
                           optimistic_slow_projection_ms={key: value * 1024 / skipped / 1000
                                                          for key, value in savings.items()}))
    return dict(proof=proof["setup_proof"], blocks=blocks, raw_mesh_arms=36,
                dense_effect_ms={key.removesuffix("_us"): distribution([block["savings_us"][key] / 1000 for block in blocks])
                                 for key in blocks[0]["savings_us"]},
                optimistic_slow_ms={key.removesuffix("_us"): distribution([block["optimistic_slow_projection_ms"][key] for block in blocks])
                                    for key in blocks[0]["savings_us"]})


def wave_readout(archive, ledger):
    jobs = ledger["jobs"]
    origin, = ledger["transactions"]
    commit = origin["committed_us"]
    assert len(jobs) == JOBS and all(j["hold_ms"] == 100 for j in jobs)
    assert len({j["settled_xid"] for j in jobs}) == JOBS
    app_xids = {j[field] for j in jobs for field in ("attempt_xid", "effect_xid")}
    assert len(app_xids) == JOBS * 2
    assert all(j["settled_xid"] not in app_xids for j in jobs)
    assert str(int(origin["xid"]) % (2**32)) not in app_xids | {j["settled_xid"] for j in jobs}
    probe = app_sql(archive, JOBS)
    assert probe["calls"] == JOBS * 2
    prewrite = [j["attempt_committed_us"] - j["entered_us"] for j in jobs]
    surplus = [j["effect_committed_us"] - j["attempt_committed_us"] - HOLD_US for j in jobs]
    assert min(surplus) >= 0
    assert min(prewrite) >= 0
    clocks = {}
    for field in ("claimed_us", "entered_us", "attempt_committed_us", "effect_committed_us", "settled_us"):
        by_rank = sorted(j[field] - commit for j in jobs)
        clocks[field] = [by_rank[rank - 1] / 1000 for rank in range(SLOTS, JOBS + 1, SLOTS)]
    effects, settlements = (clocks[k] for k in ("effect_committed_us", "settled_us"))
    return dict(probe=probe, warm_application_sessions=8, rank_step=SLOTS, wave_offsets_ms=clocks,
                hold_floor_ms=JOBS * HOLD_US / SLOTS / 1000,
                first_write_service_slot_normalized_ms=sum(prewrite) / SLOTS / 1000,
                over_hold_slot_normalized_ms=sum(surplus) / SLOTS / 1000,
                combined_app_interval_slot_normalized_ms=(sum(prewrite) + sum(surplus)) / SLOTS / 1000,
                first_write_rank_64_to_512_ms=clocks["attempt_committed_us"][-1] - clocks["attempt_committed_us"][0],
                last_effect_to_last_settlement_ms=settlements[-1] - effects[-1])


def slow_result(dense):
    stock = history.stock_report()
    assert stock == run.read_json(SLOW / "evidence/history.json")["stock"]
    measured = []
    optimistic = dense["optimistic_slow_ms"]["drain"]
    for block in stock:
        group = block["group"]
        path, = (history.STOCK / "blocks").glob(f"{group:02d}-slow.*/measured-product/measured-mesh/raw.tar.gz")
        with tarfile.open(path) as raw:
            ledger = image(raw, 1)
            assert ledger["database"]["max_connections"] == 68 and ledger["queue_policy"] == "set_based"
            assert json_member(raw, "queue-policy")["terminal_xids"] == JOBS
            price = wave_readout(raw, ledger)
        gap = block["mesh_minus_peer_ms"]["river"]
        required = max(0, gap["submit_ms"])
        measured.append(dict(group=group, raw_sha256=run.digest(path), river_gap_ms=gap, **price,
                             held_producer_fixed_required_drain_saving_ms=required,
                             required_saving_per_wave_ms=required / 8,
                             required_us_per_probe_if_all_removed=required * 1000 / 1024,
                             submit_gap_after_best_projection_ms=gap["submit_ms"] - optimistic["max"]))
    losers = [row for row in measured if row["river_gap_ms"]["submit_ms"] > 0]
    assert [x["group"] for x in losers] == [1, 2, 5]
    assert all(r["held_producer_fixed_required_drain_saving_ms"] > optimistic["max"] for r in losers)
    return measured


def cold_pilot():
    command = [sys.executable, "-B", str(RECOVERY / "controller.py"), "replay", "pilot"]
    replay = subprocess.check_output(command, cwd=ROOT)
    assert replay == (RECOVERY / "evidence/native/pilot/replay.json").read_bytes()
    source = RECOVERY / "evidence/native/pilot"
    report = json.loads(replay)
    blocks = []
    for group, gate in zip(report["groups"], report["gates"], strict=True):
        measured = {}
        for phase in ("warmup", "measured"):
            for lane in ("inline-a", "inline-b"):
                record = group["scores"][phase][lane]
                path = source / f"blocks/{group['path']}/{phase}-{lane}/{phase}-mesh/raw.tar.gz"
                assert run.digest(path) == record["raw_sha256"]
                with tarfile.open(path) as raw:
                    ledger = image(raw, 1)
                    assert record["probes"] == 1024
                    assert record["exact_ready_and_before_submit"] == 8
                    price = wave_readout(raw, ledger)
                if phase == "measured":
                    measured[lane] = dict(raw_sha256=record["raw_sha256"], **price)
        blocks.append(dict(group=group["group"], measured=measured,
                           duplicate_spread_ms=gate["duplicate_spread_ms"]))
    return blocks


def opt_in(dense):
    full = PUBLIC / "evidence/native/setup/trusted-jobs-product-623c46b92cc5"
    source_path = f"source/tools/durable-jobs-bench/results/{PUBLIC.name}"
    assert run.read_json(full / "controller-release.json")["exit_status"] == 0
    assert run.digest(full / "mesh.json") == MANIFEST
    binding = run.read_json(DENSE / "evidence/closure-retry-binding.json")["original"]
    assert binding["build"]["files"]["mesh"] == ELF and binding["product"] == REVISION
    assert run.digest(PUBLIC / "source.tar.gz") == binding["source_sha256"]
    assert run.digest(full / "raw.tar.gz") == binding["setup_sha256"]
    assert run.digest(full / "controller-release.json") == binding["release_sha256"]
    sha = hashlib.sha256
    with tarfile.open(PUBLIC / "source.tar.gz") as source:
        def read_member(path):
            return source.extractfile(path).read()

        pool = read_member("source/compiler/mesh-rt/src/db/pool.rs")
        fixture = read_member(source_path + "/fixture/main.mpl")
        semantic = read_member(source_path + "/replay/semantic.py")
        assert b'param == "mesh_pool_hot_checkout=1"' in pool
        assert b"Duration::from_millis(500)" in pool and b'pg_simple_command(conn, "SELECT 1")' in pool
        assert b'Pool.open(Env.get("MESH_EFFECT_APP_URL", ""), lanes, lanes, 5000)' in fixture
        assert b'os.environ["PRODUCT_COMPOSITION"] = "product-d2"' in semantic
    with tarfile.open(full / "raw.tar.gz") as raw:
        fault_path = "./semantic-root/semantic/ambiguous-application-write/result.json"
        fault = raw.extractfile(fault_path).read()
        clean = raw.extractfile("./semantic-root/semantic/public-handler/result.json").read()
    assert sha(fault).hexdigest() == dense["proof"]["ambiguous-application-write"]
    assert sha(clean).hexdigest() == dense["proof"]["public-handler"]
    fault = json.loads(fault)
    assert fault["wire"] == dict(command="INSERT 0 1", receipt_dropped=True, rows=0, idle=True)
    assert fault["app_insert_calls"] == 3 and fault["final"]["state"] == "completed"
    stock = run.read_json(history.BASELINE / "slow.inventory.json")["configuration"]["lanes"]["product"]
    argv = stock["inventory"]["engines"]["mesh"]["argv"]
    assert "PRODUCT_COMPOSITION=product-bulk" in argv and not any("checkout" in arg.lower() for arg in argv)
    backend = (PUBLIC / "closure-retry/backend.py").read_text()
    assert '"product-bulk": ("set_based", False)' in backend
    assert '"product-d2": ("success_cohort", True)' in backend
    assert '&mesh_pool_hot_checkout=1' in backend
    return dict(revision=REVISION, executable_sha256=ELF, manifest_sha256=MANIFEST,
                source_sha256=run.digest(PUBLIC / "source.tar.gz"), native_setup_sha256=run.digest(full / "raw.tar.gz"),
                pool_source_sha256=sha(pool).hexdigest(), fixture_sha256=sha(fixture).hexdigest(),
                url_suffix="&mesh_pool_hot_checkout=1", hot_idle_threshold_ms=500,
                retained_positive_fault_policy="success_cohort", scalar_hot_native_proof=False,
                exact_binary_supports_independent_pool_opt_in=True)


def booking_scan():
    """Find exact candidates in retained series and the local controller inbox."""
    patterns = ("series.json", "*packet*.json", "*inventory*.json", "*request*.json", "*plan*.json")
    archive_paths = {path for pattern in patterns for path in RESULTS.rglob(pattern)}
    candidates = {}
    counts = {}
    for label, paths in (("archive", archive_paths), ("inbox", list(INBOX.glob("*.json")))):
        selected = {}
        for path in sorted(set(paths)):
            text = path.read_text().lower()
            if '"slow"' not in text:
                continue
            if not any(token in text for token in (
                    "mesh_pool_hot_checkout", "product-d2", "mesh_pg_hot_checkout", '"hot"')):
                continue
            # Keep the admission decision independent of narrative references.
            # A document with these tokens still needs a real hot launch lane.
            config = json.loads(text)
            actual = any(token in text for token in (
                "product_composition=product-d2", "mesh_pool_hot_checkout=1", "mesh_pg_hot_checkout=1"))
            affinity = ELF in text or REVISION in text or "trusted-jobs-product-623c46b92cc5" in text
            selected[str(path.relative_to(ROOT))] = dict(actual_hot_option_literal=actual,
                                                         exact_public_artifact_reference=affinity,
                                                         document_type=type(config).__name__)
        counts[label] = len(paths)
        candidates[label] = selected
    return dict(scanned_documents=counts, lexical_slow_hot_hits=candidates)


def emit(data):
    print(json.dumps(data, indent=2, sort_keys=True, allow_nan=False))


if __name__ == "__main__":
    if sys.argv[1:] == ["scan"]:
        emit(booking_scan())
    elif not sys.argv[1:]:
        dense = dense_result()
        emit(dict(same_image_dense=dense, canonical_slow=slow_result(dense), recent_cold_pilot=cold_pilot(),
                  opt_in=opt_in(dense),
                  decision="close without native intake",
                  reason="All three observed submit losses exceed the best all-probes-removed dense-rate projection."))
    else:
        sys.exit("expected no argument or scan")
