"""Replay same-runner batch pair capacity from the sealed canonical pilot."""
from collections import Counter, defaultdict
from hashlib import sha256
from io import BytesIO
import json
import subprocess
import tarfile

PILOT = "dad83049a5d35f03dd22e8d59c14b9b79db1679b"
ROOT = "tools/durable-jobs-bench/results/2026-11-25-public-dense-8x8"
PATH = f"{ROOT}/path-recovery/evidence/pilot"
ELF = "4924b5880247059032786f43f490cf99bde8a0a69adc790994ee01a51ab91058"
SOURCE = "3c4032d06b10bc54a47290c673562fb063b47789"

INPUT = "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"
BODY = "ecf89d59b8ffcfa3b07aea4b2c1533d329795f366ced63b7e4221a1b40040044"


def fetch(revision, path):
    return subprocess.check_output(["git", "show", f"{revision}:{path}"])


def read(path):
    return json.loads(fetch(PILOT, f"{PATH}/{path}"))


def opportunity(by_runner, advance_us, lag_us):
    earliest = [0, 0, 0]
    weighted = [0, 0, 0]
    max_weight = [0, 0, 0]
    for jobs in by_runner.values():
        stamps = sorted(Counter(j["claimed_us"] for j in jobs).items())
        candidates = {}
        for index, ((first, first_width), (second, second_width)) in enumerate(zip(stamps, stamps[1:])):
            at = first - advance_us
            occupied = sum(j["claimed_us"] < first and j["settled_us"] + lag_us > at for j in jobs)
            if occupied + first_width + second_width <= 8:
                candidates[index] = (1, second_width * (second - first), second_width)

        index = 0
        while index < len(stamps):
            if index in candidates:
                earliest = [a + b for a, b in zip(earliest, candidates[index])]
                index += 1
            index += 1

        best = [(0, 0, 0), (0, 0, 0)]
        heavy = best.copy()
        for index in range(len(stamps) - 1):
            pair = candidates.get(index, (0, 0, 0))
            best.append(max(best[-1], tuple(a + b for a, b in zip(best[-2], pair))))
            heavy.append(max(heavy[-1], tuple(a + b for a, b in zip(heavy[-2], pair)),
                             key=lambda item: (item[1], item[0], item[2])))
        weighted = [a + b for a, b in zip(weighted, best[-1])]
        max_weight = [a + b for a, b in zip(max_weight, heavy[-1])]

    def metrics(values):
        pairs, seat_us, seats = values
        return dict(pairs=pairs, seats=seats, normalized_slot_ms=seat_us / 64000)

    return dict(earliest=metrics(earliest), greatest_weight_at_maximum_pairs=metrics(weighted),
                greatest_weight=metrics(max_weight))


def replay_arm(rotation, score):
    raw = fetch(PILOT, f"{PATH}/blocks/{rotation:02}-noop/measured-B/measured-mesh/raw.tar.gz")
    digest = sha256(raw).hexdigest()
    assert digest == score["raw_sha256"]
    with tarfile.open(fileobj=BytesIO(raw)) as archive:
        def member(name):
            return json.load(archive.extractfile(f"adapter/{name}.json"))

        ledger = member("ledger")
        route = member("public-grants")["claim_batch_bounded_v1"]
        policy = member("queue-policy")
        ready = member("canonical-ready")
        build = member("build")

    assert build["version"] == SOURCE and build["files"]["mesh"] == ELF
    assert ledger["input_sha256"] == INPUT
    assert ledger["runners"] == 8 and ledger["slots"] == 64 and ready["lane"] == "bounded-grouped-hot"
    assert route["body_sha256"] == BODY and route["configuration"] == [
        "search_path=pg_catalog, pg_temp", "lock_timeout=100ms",
    ]
    assert route["worker"] and not (route["app"] or route["producer"] or route["security_definer"])

    jobs = ledger["jobs"]
    assert len(jobs) == len({j["id"] for j in jobs}) == len({j["job_key"] for j in jobs}) == 1024
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
    by_runner = defaultdict(list)
    for job in jobs:
        assert job["state"] == "completed" and job["attempts"] == job["effects"] == 1
        assert (job["claimed_us"] <= job["entered_us"] <= job["attempt_committed_us"] <
                job["effect_committed_us"] <= job["settled_us"])
        by_runner[job["worker_pid"]].append(job)
    assert len(by_runner) == 8
    assert len({j["settled_xid"] for j in jobs}) == score["terminal_xids"]
    groups = len({(j["worker_pid"], j["claimed_us"]) for j in jobs})
    query, = (q for q in policy["queue_statements"] if "FROM mesh_jobs.claim_batch_bounded_v1" in q["query"])
    assert groups == query["calls"] - (query["rows"] - len(jobs))
    return dict(
        raw_sha256=digest, grants=groups, all_bounded_calls=query["calls"],
        bounded_server_elapsed_ms=round(query["execution_ms"], 3),
        all_calls_removal_for_extra_exchange_parity=(query["calls"] + 3) // 4,
        submit_us=score["submit_us"], drain_us=score["drain_us"],
        sizes=dict(sorted(Counter(Counter(
            (job["worker_pid"], job["claimed_us"]) for job in jobs
        ).values()).items())),
        advance_sensitivity={ms: opportunity(by_runner, ms * 1000, 0)
                             for ms in (0, 2, 4, 6, 8, 12, 16, 24)},
        confirmation_lag_sensitivity_at_4ms={ms: opportunity(by_runner, 4000, ms * 1000)
                                             for ms in (0, 1, 2, 4, 8, 12, 16, 24)},
    )


def main():
    release = read("controller-release.json")
    assert release["exit_status"] == 0
    replay = read("replay.json")
    assert replay["advance_to_peers"]
    return {rotation: replay_arm(rotation, replay["groups"][str(rotation)]["measured-B"])
            for rotation in (1, 2)}


if __name__ == "__main__":
    print(json.dumps(main(), indent=2, sort_keys=True))
