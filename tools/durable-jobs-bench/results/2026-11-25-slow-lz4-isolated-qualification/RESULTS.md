# Matched Linode PGLZ and LZ4 results

The build-free continuation released exit 0 at 17:57:27 UTC on November 25, 2026. Independent replay matched the controller's score and reconciled all 80 selected arms across five frozen rotations. Rotations 1–2 came from the original grant. Fresh rotations 3–5 came from the continuation. The nine captures from the interrupted rotation 3 are preserved outside the comparison. [Original failed release](evidence/native/recovery-3/timing/controller-release.json) · [Continuation release](evidence/native/recovery-3/timing-continuation/controller-release.json) · [Full paired scores](evidence/native/recovery-3/timing-continuation/independent-replay.json)

All selected arms used the same isolated PostgreSQL 17.11 executable, AMD64 Linode boot, disk and CPU pair. Each arm reconciled 512 canonical payloads, both independent application transactions per job and the terminal receipts. Every LZ4 arm stored 512 compressed payloads. Every PGLZ arm stored 512 uncompressed payloads. Peak sampled CPU steal was 0.28% across the scored windows. [Qualified inventory](recovery-3/matched-inventory.json)

## Codec effects within each engine

Values are the paired median [minimum, maximum] in milliseconds across five rotations. The sign is PGLZ minus LZ4. Positive values favor LZ4.

| Engine | Submit to producer commit | Submit to last settlement | Commit to last settlement |
| --- | ---: | ---: | ---: |
| Mesh V1 bulk | +162.6 [+138.3, +245.8] | +126.5 [+55.3, +326.5] | -36.1 [-83.0, +80.7] |
| River | +161.9 [+143.8, +218.2] | +10.0 [+0.4, +263.9] | -139.6 [-170.4, +45.7] |
| Oban | +159.5 [+106.1, +248.1] | +242.8 [+196.1, +276.8] | +83.3 [+20.1, +114.6] |
| Apalis | +177.8 [+159.3, +189.8] | +104.3 [+1.1, +229.0] | -59.1 [-188.6, +44.7] |
LZ4 shortened submit to producer commit and submit to last settlement in all five rotations for each engine. River's full-clock savings ranged from 0.4 to 263.9 ms. For Mesh, the median full-clock saving was 126.5 ms, with 162.6 ms saved to producer commit. Mesh's commit-to-settlement window was longer under LZ4 in three of five rotations. Its 64th-to-final first-application-commit interval was 31.2 ms longer by paired median, with a range from 105.0 ms shorter to 61.8 ms longer. These landmarks describe write progress in the measured runs.

## Mesh versus peers within each codec

Values are the paired median [minimum, maximum] in milliseconds. The sign is Mesh minus peer. Positive values mean Mesh took longer.

| Codec | Peer | Submit to last settlement | Commit to last settlement |
| --- | --- | ---: | ---: |
| PGLZ | River | +126.3 [+96.8, +492.3] | +105.9 [+41.2, +432.2] |
| PGLZ | Oban | -137.7 [-232.6, +219.5] | -151.1 [-211.0, +151.7] |
| PGLZ | Apalis | +546.4 [+392.5, +841.9] | +375.3 [+258.1, +654.3] |
| LZ4 | River | +60.6 [-25.6, +376.4] | +6.1 [-60.3, +363.8] |
| LZ4 | Oban | -42.0 [-91.0, +312.1] | -40.8 [-117.8, +280.5] |
| LZ4 | Apalis | +488.7 [+347.0, +945.0] | +296.5 [+152.4, +764.4] |
Apalis finished ahead of Mesh on both clocks in all five rotations under both codecs. River finished ahead in all five PGLZ rotations and three of five LZ4 rotations on both clocks. Mesh finished ahead of Oban in three of five rotations on the submit clock and four of five on the commit clock under either codec.

The Mesh arm is the frozen V1 bulk producer with the `claim_batch_v1` cursor. Public compressed-claim candidates and the historical DEFAULT/EXTERNAL comparison have their own evidence. [Original workload](../2026-11-23-slow-storage-transfer/README.md) · [Separate DEFAULT/EXTERNAL results](../2026-11-24-slow-external-compat-scorer-input/README.md)
