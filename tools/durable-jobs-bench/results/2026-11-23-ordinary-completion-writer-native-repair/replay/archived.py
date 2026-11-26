"""Independently reconcile the failed r2 archive before qualifying a build-free retry."""
from collections import Counter
import hashlib
import json
from pathlib import Path
import tarfile

ORIGINAL = Path(__file__).resolve().parents[2] / "2026-11-23-ordinary-completion-writer/combined"
FIRST = ORIGINAL / "evidence/bulk-writer-b1dcd34af288"
PACKET_SHA = "87334bda19cd2f0e97a81f937b632bfce965d9804fd234375bbe820870c23b36"
RAW_SHA = "eb39628d011afa879b0205dc6333ec53fd16fb05eefb8333d3f896f6ac9887ed"
MANIFEST_SHA = "361f10d2c2866f694989198bb8359d29c91dd9928dcd95e8f92293293f226d29"
WITNESS_SHA = "277f42270ed293b9bc5384aa650f6baf0d667ab022e5e81daf663b423a8c0440"
BROKER_SHA = "55d7000d562a6ad395f5e64e33e0e973fac0084ed05ad6c30a33db2a3fc6303a"
RUNNER_SHA = "93fef96704ae412cf3c55fae73cc2aac405c8b423670826b4876954a44c8b010"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read(raw, name):
    members = [entry for entry in raw.getmembers() if entry.name == "./" + name]
    (member,) = members
    assert member.isfile(), name
    with raw.extractfile(member) as stream:
        return stream.read()


def state(row, status, attempts, effects=1, *, recorded=None):
    assert row["state"] == status and row["attempt"] == attempts and row["effects"] == effects
    assert row["attempts"] == (list(range(1, attempts + 1)) if recorded is None else recorded)
    assert row["id"] and row["xid"].isdecimal()


def fault_checks(cases):
    for name in ("held-middle", "expired-middle", "cancelled-member", "ambiguous-completion"):
        case = cases[name]
        assert case["during_receipt_hold"]["queues"]["default"]["in_flight"] == 4
        assert Counter(row[0] for row in case["held_physical_backends"]) == {
            "parity_subject": 3, "compiled_caller": 2}
        assert all(row[2] for row in case["held_physical_backends"])
        final = case["final"]
        for key in ("left", "right", "prime"):
            state(final[key], "completed", 1)
        assert final["left"]["xid"] == final["right"]["xid"]
        state(final["later-retry"], "completed", 2)

    for name in ("held-middle", "expired-middle"):
        case = cases[name]
        for key in ("left", "right"):
            assert case["peers_while_middle_locked"][key] == case["final"][key]
    final = cases["held-middle"]["final"]
    state(final["middle"], "completed", 1)
    assert final["middle"]["xid"] != final["left"]["xid"]
    expired = cases["expired-middle"]
    state(expired["final"]["middle"], "executing", 1)
    assert expired["final"]["middle"]["id"] in expired["local_slot_after_expiry"]["undrained_jobs"]

    final = cases["cancelled-member"]["final"]
    state(final["middle"], "cancelled", 1)
    assert final["middle"]["xid"] != final["left"]["xid"]
    case = cases["ambiguous-completion"]
    assert case["wire"] == dict(command="SELECT 3", rows=3, idle=True, receipt_dropped=True)
    cohort = case["committed_before_receipt_loss"]
    assert set(cohort) == {"left", "middle", "right"}
    assert len({row["xid"] for row in cohort.values()}) == 1
    for key, row in cohort.items():
        assert row == case["final"][key]
        state(row, "completed", 1)

    case = cases["ambiguous-claim"]
    assert case["wire"] == dict(command="COMMIT", rows=0, idle=True, receipt_dropped=True)
    assert set(case["final"]) == set(case["before_recovery"]) == {"claim-a", "claim-b"}
    assert len({row["xid"] for row in case["before_recovery"].values()}) == 1
    for key, row in case["before_recovery"].items():
        state(row, "executing", 1, 0, recorded=[])
        final = case["final"][key]
        state(final, "completed", 2, recorded=[2])
        assert final["id"] == row["id"]

    case = cases["renew-priority"]
    for key in ("left", "right"):
        state(case["renew_before_successes"][key], "executing", 1)
        state(case["final"][key], "completed", 1)
    assert case["final"]["left"]["xid"] == case["final"]["right"]["xid"]
    state(case["renew_before_successes"]["renew"], "executing", 1, 0)
    state(case["final"]["renew"], "completed", 1)
    assert case["renew_before_successes"]["renew"]["lease_expires_at_ms"] > 0

    final = cases["stop"]["final"]
    state(final["prime"], "completed", 1)
    state(final["after-stop"], "available", 0, 0)


def verify():
    packet_path = ORIGINAL / "controller-packet.json"
    raw_path = FIRST / "raw.tar.gz"
    assert digest(packet_path) == PACKET_SHA and digest(raw_path) == RAW_SHA
    packet = json.loads(packet_path.read_text())
    root = ORIGINAL.parents[4]
    for path, expected in packet["pins"].items():
        assert digest(root / path) == expected, path
    receipt = json.loads((ORIGINAL / "evidence/first-native-setup.json").read_text())
    grant, status = [json.loads((FIRST / f"{name}.json").read_text()) for name in ("grant", "status")]
    central_dest = root / "target/linode-jobs-stepchange/requests"
    central = json.loads((central_dest / f'{packet["id"]}.released.json').read_text())
    assert central == receipt["release"] and central["exit_status"] == status["exit_code"] == 1
    assert packet["id"] == grant["id"] == central["id"] and receipt["attempt"] == grant["attempt"]
    assert grant["start"] <= status["utc"] <= central["command_finished_at"] < central["released_at"] <= grant["end"]
    assert receipt["raw_sha256"] == RAW_SHA and receipt["timing_started"] is False

    with tarfile.open(raw_path, "r:gz") as raw:
        parse = lambda name: json.loads(read(raw, name))
        host = parse("host.json")
        manifest = read(raw, "runner-build.json")
        runner = json.loads(manifest)
        assert hashlib.sha256(manifest).hexdigest() == MANIFEST_SHA
        assert runner["files"]["mesh"] == receipt["runner_elf_sha256"] == RUNNER_SHA
        assert runner["source_patch_sha256"] == packet["source_patch_sha256"]
        assert runner["version"] == packet["source_sha"]
        assert runner["capsule"] == packet["id"] and runner["architecture"] == "x86_64"
        assert runner["system"] == "Linux" and runner["execution_contract"] == "ordinary-trusted-shared-process-v2"
        cases = {case: parse(f"proof/faults/{case}/result.json") for case in receipt["fault_cases"]}
        assert len(cases) == 7 and set(read(raw, "faults.log").decode().splitlines()) == {
            f"{case} passed" for case in cases}
        fault_checks(cases)

        teardowns = {}
        for mode in ("off", "zero"):
            witness = parse(f"proof/teardown-{mode}/teardown.json")
            assert witness["runner_sha256"] == WITNESS_SHA and witness["broker_sha256"] == BROKER_SHA
            assert witness["mode"] == mode and witness["normal_exit"] is True
            assert witness["callback_held_witness"] == "entered present and finished absent before release"
            held, done = witness["held_callback_state"], witness["after_callback_state"]
            assert (held["state"], done["state"]) == ("executing", "completed")
            assert held["attempt"] == done["attempt"] == 1
            assert all(held[key] == done[key] for key in ("attempt_xid", "effect_xid"))
            assert len({held["job_xid"], done["job_xid"], done["attempt_xid"], done["effect_xid"]}) == 4
            teardowns[mode] = witness

        failure = parse("smoke-off/failure.json")
        assert failure == receipt["failure"] and failure["type"] == "PermissionError"
        assert "/proc/1945661/exe" in failure["message"] and failure["stage"][2][2] == "capture_runners"
        invocation = parse("smoke-off/invocation.json")
        assert invocation == dict(engine="mesh", profile="short", phase="diagnostic", local_smoke=False,
                                  execution_contract=runner["execution_contract"], queue_policy="success_cohort")
        assert parse("smoke-off/process-affinity.json")["1945661"] == packet["cpus"]
        files = [member.name for member in raw.getmembers()]
        assert not any(name.startswith("./smoke-zero") for name in files)
        assert not any(name.endswith(("/row.json", "/ledger.json", "/native-seal.json", "/producer-writer.json"))
                       for name in files)

    return dict(attempt=grant["attempt"], packet_sha256=PACKET_SHA, raw_sha256=RAW_SHA, exit_status=1,
                host=host, runner_sha256=RUNNER_SHA, runner_manifest_sha256=MANIFEST_SHA,
                fault_cases=sorted(cases), teardown_witnesses=teardowns, off_smoke="failed before enqueue",
                zero_smoke="not started", timing_groups_started=[])


if __name__ == "__main__":
    print(json.dumps(verify(), indent=2, sort_keys=True))
