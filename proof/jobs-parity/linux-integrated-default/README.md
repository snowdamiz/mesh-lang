# Integrated default Jobs evidence

The normal GNU ARM64 ELF is `9eda8768d9347b6b9dc6cc7f45381c80bbabf15304e643203c5af5e679a2505f`. The link handoff at `f1e646f0` pins the runtime archive built with JSON `235a38cc` and PostgreSQL `a00637fe`. The retained control is `fe719b77`. The layout pilots used two distinct relinks of the current archive, the portable `db7f69c8` and the exact-input `df2c2f3c`. The receipt records full commit, ELF, input, link, guest, and scorer hashes. [Receipts](receipts.json)

The 19 completed groups contain 69 measured arms and 39 full-warmup arms. The timed arms used fresh databases, durable commits, TLS, and trace-off workers. A shared 15-core Apple Silicon host ran the two-vCPU Linux ARM64 guests. PostgreSQL was 17.11. Oban Basic 2.24.2 used the `8526a5bd` fixture and file-only resolver with Erlang 29.1.1, Elixir 1.20.4, and `+S 2:2 +SDcpu 1:1 +SDio 1 +A 2`. Preflight, live runners, and the shared broker verify the resolver. Resource metrics cover the runner tree, including sampled shared RSS and startup/idle CPU. The queue and exact-layout receipts also carry shutdown-inclusive peaks.

| Profile | Groups × measured arms | Jobs per arm | Runners / slots / connections | Measured / full-warmup jobs | Result |
| --- | ---: | ---: | ---: | ---: | --- |
| Sparse, 1 KiB | 5 × 3 | 24 | 1 / 1 / 5 | 360 / 0 | Current and retained pass the five-group score |
| Short, 1 KiB | 5 × 3 | 128 | 1 / 16 / 20 | 1,920 / 0 | Current and retained pass the five-group score |
| Large, 512 KiB | 2 × 4 | 64 | 8 / 64 / 96 | 512 / 512 | Both preselected current arms miss rate and pickup |
| Dense no-op | 3 × 5 | 1,024 | 8 / 64 / 96 | 15,360 / 15,360 | First forward pass for current and portable layout, then two rate and pickup misses |
| Slow, portable layout | 2 × 5 | 512 | 1 / 64 / 68 | 5,120 / 5,120 | Every Mesh arm misses the same-boot RSS cap twice |
| Slow, exact-input layout | 2 × 3 | 512 | 1 / 64 / 68 | 3,072 / 3,072 | One matched 64-hold RSS pass. Both pilots miss rate |

**Sparse and short.** Current sparse reached 2.819765 durable jobs/s versus Oban's 2.020441. Current/Oban ratios for rate, pickup p99, CPU, and RSS were 1.396×, 0.081×, 0.499×, and 0.270×. The worst current claim p99 was 50.155 ms against 100 ms. Each arm settled 120 jobs across the five groups. Current short reached 962.289431 jobs/s versus 409.857121, with ratios of 2.348×, 0.419×, 0.286×, and 0.423×. Each arm settled 640 jobs. Keep the standalone current rate miss in short-03 and the rate and pickup misses in short-04 in the aggregate. Sparse and short used idle staging. Each sparse cohort had 24 enqueue commits, 48 distinct application XIDs, and 24 settlement XIDs. Each short cohort had one enqueue commit, 256 distinct application XIDs, and 128 settlement XIDs. Mesh had 1 and 8 startup resets, respectively, plus one reset per application write.

**Large and dense controls.** The same two byte-identical current-normal arms measured 51.291 and 42.532 jobs/s forward, then 41.922 and 214.209 in reverse. Only the reverse repeat cleared the large gates. The retained arm cleared them in both boots. Each large arm, including warmup, reconciled 33,554,432 payload bytes by per-job hash, one enqueue XID, 128 application XIDs, and 64 settlements. In dense, the same current-normal arm and its repeat measured 917.12 and 243.98, 179.99 and 259.14, then 166.26 and 291.82 jobs/s. The faster duplicate was 1.44× to 3.76× the slower duplicate. Current normal and the portable layout passed in the first forward boot. All Mesh arms missed rate and pickup in reverse and in the lower-start-load forward boot. The duplicate spreads leave the combined runtime and layout effects unresolved under these loads.

**Slow layouts.** The two portable-layout rotations reached 64 committed holds in every measured arm. The portable layout saved 50.5–52.3 MiB against the two byte-identical normal controls in the corresponding boots and beat same-boot Oban on rate twice. Its RSS still exceeded the shutdown-inclusive Oban caps by 70.5–75.7 MiB. Both ELF identities separately passed the held lifecycle with 64 joined fresh execs, zero exits, and exact consuming waits per traced ELF. Each held arm recorded 128 independent application XIDs, 128 per-call resets, and eight startup resets.

The separate `df2c2f3c` exact-input layout reproduced the stock ELF, candidate ELF, maps, and sidecars on a clean relink. Training captured 495 of 512 settled workers and independent validation captured 498 of 512. The separate traced lifecycle reconciled 64 fresh execs, zero exits, and exact consuming waits per ELF. The held peak of 219.33 MiB exceeded its predeclared 203.016 MiB cap. In the timed forward boot the candidate reached 47 of 64 committed holds, normal reached 51, and Oban reached 64. In reverse all three reached 64. The candidate used 213.47 MiB against Oban caps of 214.71 MiB during measurement and 215.08 MiB through shutdown. It ran at 319.29 jobs/s versus Oban's 322.30. The forward pilot also missed pickup. The combined two-pilot calculation misses rate and pickup, and its RSS median includes the partial-overlap forward arm. Both layouts stopped before five pairs.

**Failed attempts.** Sparse-02's first reverse attempt stopped before timing at Oban broker readiness. Its complete same-order retry contributes the score. The initial dense observer failed on `/proc/PID/exe`. Its retry hit the watchdog before a terminal receipt. Two queue held attempts and two exact-layout training attempts stopped before boot. The first 4 KiB training boot exited with status 1 after timing out on the residency report. The archived diagnosis attributes the observer exit to a worker disappearing during a status read. The excluded-attempt entries retain the scratch file hashes and reasons. All completed groups in the table, including the exact-layout partial-overlap result, contribute to their stated scores.

**Standalone runtime measurements.** Five interleaved ordinary Mesh JSON runs per arm cut dense-batch parse CPU from 750 to 404 µs. The 512 KiB parse/get/encode ranges overlapped. In the separate direct PostgreSQL 17.11 client replay, median plain short-query/update CPU fell from 42.61 to 25.33 ms across five alternating triplets. Verified TLS CPU had no consistent paired direction. These measurements isolate ordinary JSON and client work on their respective workloads. The queue results above measure the integrated ELF.

**Earlier results.** Under the earlier corrected stock series, `fe719b77` qualified on short only. That series still has a 143.19 ms sparse claim miss in sparse-05. The older `ba80441a` packaging passed the five-pair frozen slow score, with a separate matched-occupancy miss of 61 out of 64 in slow-04. [Corrected stock score](../linux-canonical-inetrc-score-20261122.json) · [Older slow layout proof](../linux-worker-layout/README.md)

## Recalculate the retained scores

Run from the checkout root with Python 3.11 or later. The replay checks the pinned scorer and profile files, then uses their existing functions. It compares the complete score summary and each gate verdict to the reductions made from the original guest receipts. Pilot summaries use the same frozen limits with their recorded group counts. The replay uses the retained metrics. Guest logs, payloads, and ELF bytes stay in the host scratch directories.

```sh
python3 -B - <<'PY'
from hashlib import sha256
import json
from pathlib import Path
import runpy

record = json.loads(Path("proof/jobs-parity/linux-integrated-default/receipts.json").read_bytes())
for name, digest in record["pins"]["scorer_sha256"].items():
    assert sha256(Path(name).read_bytes()).hexdigest() == digest, name

scorer = runpy.run_path("scripts/jobs-parity.py")
settings = json.loads(Path("scripts/jobs-parity-profiles.json").read_bytes())

def check(suite, groups, scores, scope):
    profile, slots = suite["profile"], suite["slots"]
    for label, modes in scores.items():
        for mode, expected in modes.items():
            trials = []
            for pair, group in enumerate(groups):
                for arm_name, engine in ((label, "mesh"), ("oban", "oban")):
                    arm = group["measured"][arm_name]
                    metrics = dict(arm["metrics"], engine=engine, pair=pair)
                    if mode == "through_shutdown":
                        metrics.update(arm["shutdown_inclusive"])
                    assert metrics["completed"] == suite["jobs_per_arm"], scope
                    assert metrics["input_sha256"] == suite["input_sha256"], scope
                    assert metrics["jobs_per_second"] == metrics["completed"] / arm["durable_seconds"], scope
                    trials.append(metrics)
            scorer["validate_pairs"](trials, len(groups), slots)
            summary = scorer["profile_summary"](
                trials, profile, settings["workloads"][profile], settings, suite["runners"],
                slots, ("mesh", "oban"), True, True, None,
            )
            assert summary["budgets"]["peak_connections"] == suite["connection_cap"], scope
            misses = [f"{area}.{metric}" for area, values in summary.items()
                      for metric, rule in values.items() if isinstance(rule, dict)
                      and (rule.get("within_limit") is False or rule.get("within_budget") is False)]
            canonical = json.dumps(summary, sort_keys=True, separators=(",", ":"), allow_nan=False)
            assert sha256(canonical.encode()).hexdigest() == expected["summary_sha256"], (scope, label, mode)
            assert misses == expected["misses"], (scope, label, mode)
            if scope == "aggregate":
                mesh = summary["mesh"]["jobs_per_second"]["median"]
                oban = summary["oban"]["jobs_per_second"]["median"]
                print(f"{label} {mode}, {mesh:.6f} / {oban:.6f} jobs/s, {', '.join(misses) or 'all limits pass'}")

for name, suite in record["suites"].items():
    groups = suite["groups"]
    print(f"{name} ({len(groups)} groups)")
    check(suite, groups, suite["aggregate"], "aggregate")
    for group in groups:
        check(suite, [group], suite["per_boot"][group["name"]], group["name"])
PY
```

## Resume on the measurement host

From the checkout root, read the preserved launch and reduction commands for the needed lane.

```sh
cat target/integrated-runtime-sparse-large/REPLAY.md
cat target/integrated-runtime-queue/REPLAY.md
cat target/current-image-worker-layout/REPLAY.md
cat target/dense-retained-cpu-live/RESUME.md
```

The queue checksum list uses one space between the digest and the path. To check it with macOS `shasum`, normalize the separator.

```sh
(cd target/integrated-runtime-queue && awk '{print $1 "  " $2}' SHA256SUMS | shasum -a 256 -c -)
```

The queue reducers write comparison files. Use fresh output paths for new attempts and preserve the archived failures. Dense still needs quiet-host matched attribution with exact interior application CPU endpoints. Large still needs a repeatable rate and pickup result outside the identical-control spread. Slow needs repeatable full-overlap RSS headroom and a throughput pass in the same candidate.
