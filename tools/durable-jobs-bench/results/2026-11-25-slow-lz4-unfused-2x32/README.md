# Unfused SLOW 2×32 on the isolated PostgreSQL 17.11 image

The retained binaries support a build-free stock composition for all four engines. Route the correctness-only proof first. The combined adapter has had a source and sealed-inventory review. Its native behavior awaits the controller proof.

## What the released blocks show

The eight equal 2×32 cap-71 PGLZ rotations put Mesh ahead of River in 6/8 submit and 7/8 commit comparisons, ahead of Oban on both clocks in 8/8, and behind Apalis in 8/8. The separately released cap-68 mixed comparison ran 2×32 Mesh against 1×64 peers. It gave Mesh 5/6 against River and Oban on each clock. The isolated 1×64 codec series instead put Mesh 60.6 ms behind River on the LZ4 submit clock by paired median. The Mesh margin over Oban moved from 137.7 to 42.0 ms on that clock. Those are different deployment and codec contrasts. [Equal cap-71 results](../2026-11-24-ordinary-slow-equal-topology-cap71/NATIVE-RESULTS.md) · [Mixed cap-68 results](../2026-11-23-ordinary-slow-runner-concentration/peer-outcomes.md) · [Isolated 1×64 results](../2026-11-25-slow-lz4-isolated-qualification/RESULTS.md)

The released 1×64 PGLZ-to-LZ4 median savings for submit and commit to terminal respectively were Mesh 126.5/−36.1 ms, River 10.0/−139.6 ms, Oban 242.8/+83.3 ms, and Apalis 104.3/−59.1 ms. The package retains each actual signed pair from all three series. [Per-order released distributions and input digests](released-comparisons.json)

The 16 SHA-bound equal cap-71 Mesh/River ledgers give three River-minus-Mesh phase intervals. Commit to the 64th durable first write was +232.2 ms median [+211.2, +263.2]. The 64th first write to the last effect was −187.3 ms [−283.1, −125.8]. Last effect to terminal was +151.0 ms [+15.9, +241.6]. River reported seven idle TLS connections at both observations in every arm and a sampled peak of 68–69. These intervals locate the observed differences in this roster. The same three boundaries are independently derived from raw jobs for each fresh arm.

The physical PostgreSQL connection logs locate six River runner TLS sessions before enqueue commit and 61–62 first received and authorized after commit in each released block. By first-write rank 64, 58–60 of the post-commit sessions were already authorized. Block 7 has a one-session timestamp rounding ambiguity. The new stock arms retain `physical-postgres.log` and the same lazy pool. This evidence describes connection timing. The scores use the observed durable clocks directly. [Physical connection placement at 508e215ac](../2026-11-24-ordinary-slow-equal-topology-cap71/HOLD-PROGRESS.md)

## Experiment and gates

- The primary comparison is four engines × two codecs with two real 32-slot runner processes and PostgreSQL cap 71. Each engine uses the identical image and launch policy in both codec cells. The same isolated PG executable and extension pins are verified by the runner. Mesh uses the retained bulk producer, unfused V1 cursor, eight warm application TLS sessions, and the stock terminal path.
- Four contemporaneous PGLZ bridge cells use the released 1×64 cap-68 inventory on that same private PostgreSQL image. These bridge scores measure the deployment change, including cap and the River binary's compiled profile. They do not estimate an independent topology effect or a fresh topology-by-codec interaction.
- Each arm submits the same 512 canonical 16 KiB jobs with 100 ms committed holds. Use CPUs 0 and 1 and preserve engine-specific roles, receipt layout, queue and listener lifecycles. The scorer binds two runner PIDs and 32 slots per runner, physical TLS and role census, distinct write transactions, claim and terminal SQL, and strict 512-row compression evidence. Existing raw export is redacted, sorted and deterministic. An unfinished or rejected arm leaves its raw receipts and excludes its rotation from paired summaries. There is no automatic rerun.
- The measured clocks start at submit and durable enqueue commit and share the last durable settlement endpoint. The recorder exports a separate monotonic full-arm envelope. At 2×32 the adapter also records monotonic PG setup and runner readiness observations. For River it watches the two stock ready markers from CPU 2 before submit, at a 20 ms poll interval, without opening a SQL connection. That duration starts at Go adapter dispatch and includes migration. Record the 7-client idle snapshots and in-load peak from the stock samples. Large timestamp fields, actual phase order, warmup receipts, disk, affinity and the original noise gates stay in the raw exports.
- The retained cap-71 SLOW River ELF resets pool minima to zero. Neither the retained DENSE nor SHORT16 readiness ELF can run the sealed SLOW 2×32 cap-71 parent profile. The stock River measurement is the deployed lazy-pool roster. An explicitly warmed River claim needs a separate exact SLOW ELF and same-binary lazy/warm correctness and timing receipts. No DENSE readiness savings are applied to SLOW.

The cap-71 stock artifact path comes from a released raw inventory. The parent/controller should confirm the retained path is present before scheduling. If it was collected, its exact ELF is the member `river-slow-2x32-cap71` of the [retained cap-71 archive](../2026-11-24-ordinary-slow-equal-topology-cap71/evidence/river-linux-amd64.tar.gz), SHA-256 `a23130f74210946291ed3609797e5b218ece5a7bd70df318851cf91e634da1e1`. Repinning a different host path requires a new inventory seal. The target PostgreSQL executable SHA-256 is `3f273b467aa57364f603a6758de24f1937098bddfcd5ef8b65c90df9caaec4b8`.

Source, inventory and packet generation read the released records only. No native slot was opened. The journal scan found the separate released stock cap-71, 1×64 codec and mixed cap-68 work, plus the distinct cap-71 fused hold. It found no exact unfused 2×32 codec grant.

## Parent routing

The sole controller may first admit only `request-proof.json`, full-size 12-arm SETUP. A released exit-zero proof and complete replay are prerequisites for `request-pilot.json`, the first two reflected rotations with 48 fresh arms. The pilot likewise gates `request-full.json`, the remaining four reflected rotations with 96 fresh arms. Each timing rotation repeats its declared order for unscored warmups and fresh measurements. The scorer reports the per-order signed codec effects, Mesh-minus-peer clocks and PGLZ deployment bridge distributions separately. A failed proof or pilot leaves downstream timing gated.

Commands in the requests require the exact controller inbox bytes, live matching grant and existing local, active-arm and native fences. Only the controller stages the sealed source members or contacts Linode. The parent can route the proof command from the checkout as follows.

```sh
p=tools/durable-jobs-bench/results/2026-11-25-slow-lz4-unfused-2x32
python3 -B "$p/controller.py" proof
```

Use the corresponding packet commands for `pilot` and `full` when eligible. The `replay-proof`, `replay-pilot`, `replay-full` and `score` actions read exported archives. [Exact file seals](SHA256SUMS)
