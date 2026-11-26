# Bulk V2 completion-writer r2 native identity retry

The original SETUP ended with exit 1. Its off smoke reached live runner capture and stopped on the kernel's denial of `/proc/1945661/exe` before the first enqueue. The raw archive independently verifies the seven native fault cases and two native teardown witnesses. Both witnesses held the job in `executing` until the actor teardown callback returned and preserved its two prior application XIDs. The V2, r1 and public-product teardown comparisons in the supplementary receipt ran locally on macOS.

- [Original packet](../2026-11-23-ordinary-completion-writer/combined/controller-packet.json)
- [Original release and raw reduction](../2026-11-23-ordinary-completion-writer/combined/evidence/first-native-setup.json)
- [Original raw archive](../2026-11-23-ordinary-completion-writer/combined/evidence/bulk-writer-b1dcd34af288/raw.tar.gz)
- [Independent first SETUP replay](evidence/archived-setup-replay.json)
- [Local teardown comparison](evidence/local-teardown-replay.json)

The repair keeps the direct CPU-pinned runner and the existing broker and migrator commands. The adapter records each direct Popen handle and kernel parent/start time at launch. At readiness it checks the full live argument vector, CPU mask, and the pinned ELF on disk between two live kernel start observations. The ELF and its parent directories must be unwritable to the benchmark user. A readable executable link must identify that file. An access denial records its errno and the owned-launch method, with a null observed executable. This method trusts the sealed direct launcher and the root-managed build directory. The verifier runs as the benchmark user.

The local Linux guest used synthetic ELFs and a simulated PostgreSQL role snapshot to exercise the real pinned broker launch, direct runner launch, and readiness verifier under UID 65534 with zero effective capabilities. Linux denied both executable link reads. The verifier accepted readiness with two through four queue connections and refused five. Additional controls exercised digest and inode substitution, kernel and Popen argument changes, wrong parent, CPU drift, changed start ticks, a dead child, a mismatched readable executable, and a writable image.

- [Linux readiness and refusal receipt](evidence/linux-controls.json)
- [Exact Linux console output](evidence/linux-controls.serial)
- [Fresh controller packet](controller-packet.json)
- [Offline plan comparison](evidence/offline-plan-check.json)

The controller released SETUP at 23:47:21 UTC with exit 0. It reused the pinned runner and broker files. Each smoke completed 128 jobs in one bulk enqueue, recorded 256 independent application XIDs, and peaked at 13 physical connections under the cap of 20. The off arm recorded 122 terminal XIDs and zero recorded 61. These are setup diagnostics. Both modes recorded errno 13 for the broker and runner executable links.

- [Native SETUP qualification](evidence/native-qualification.json)
- [Native SETUP raw archive](evidence/bulk-writer-retry-36b2eeee6f07/raw.tar.gz)

The build-free request rechecks the retained Linux runner ELF and manifest, host identity, clock ticks, and the prior bulk and peer inventory. It requires fresh off and zero Linux short smokes before the original five matched timing groups can run. The four comparison lane inventories and all five group orders match the original plan. The original timing reducer calculates the scores.

From the checkout root, verify the archive and request offline.

```sh
r=tools/durable-jobs-bench/results/2026-11-23-ordinary-completion-writer-native-repair
python3 -B "$r/replay/archived.py" | cmp - "$r/evidence/archived-setup-replay.json"
python3 -B "$r/replay/controller.py" check | cmp - "$r/evidence/offline-plan-check.json"
```

Once the central controller has returned the native archives, `controller.py verify-setup` reconciles the two smoke archives against their saved digest and inventory. `verify-timing.py` replays every available raw matched group, compares the recomputed scores and digests against the saved group receipts, and reports pending groups.

## Matched short results

The controller released all five groups with exit 0. The offline replay matched all 30 raw archive digests to their recorded values. Every arm finished 128 jobs with zero failures. All ten measured writer arms recorded the owned-launch method and errno 13 for both direct children.

The same-ELF comparison showed fewer terminal XIDs with zero hold, from a median of 115 with the writer off to 74 with it on. The paired throughput change was a median of +0.6% from first submit to last settlement, with a range of −2.7% to +33.6%. From first enqueue commit to last settlement, the paired change was +3.1%, with a range of −8.3% to +30.9%. The direction varied across groups. This run did not isolate a throughput limiter.

- [All five groups, raw digests, and central release receipts](evidence/verified-timing.json)
- [Frozen reducer's summary](evidence/timing-summary.json)
