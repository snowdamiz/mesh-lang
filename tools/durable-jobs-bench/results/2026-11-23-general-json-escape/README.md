# Ordinary Mesh Json.encode string experiment

The experiment ended at the component screen. The most balanced wide scan reduced process CPU by 35% on plain Unicode and 29% on Unicode with sparse escapes. It increased CPU by 20% on dense controls and 15% when dense controls followed a plain prefix. Every fast variant tested with dense controls regressed on that workload. The broker's warm PostgreSQL runs showed a producer CPU reduction and overlapping submit-to-commit times.

## Compiled component measurements

The table shows process CPU in milliseconds per sample. Each cell gives the median and range across eight interleaved rotations. A and B run the same release executable in fresh processes. The percentage compares the candidate with the mean of the two control medians. Output size counts complete encoded trees or encoded broker specs per sample, including any repeat encodes in the frozen collector.

| Workload | Encoded MiB | A CPU | Wide scan CPU | B CPU | CPU change |
| --- | ---: | ---: | ---: | ---: | ---: |
| Plain Unicode | 166.73 | 88.34 (85.75 to 94.60) | 57.52 (54.10 to 59.88) | 89.19 (87.65 to 94.38) | -35.2% |
| Sparse escapes in Unicode | 139.08 | 91.88 (89.38 to 96.74) | 65.81 (63.10 to 73.43) | 93.72 (89.44 to 94.90) | -29.1% |
| Dense escape bursts | 76.79 | 73.79 (68.57 to 84.81) | 70.98 (61.52 to 82.12) | 70.94 (63.63 to 84.06) | -1.9% |
| Dense controls | 29.23 | 59.48 (54.62 to 62.34) | 71.96 (65.40 to 80.40) | 60.14 (55.55 to 65.27) | +20.3% |
| Dense controls after a plain prefix | 34.16 | 64.50 (60.12 to 72.22) | 74.12 (69.45 to 82.41) | 64.13 (55.99 to 68.70) | +15.2% |
| Small trees | 28.95 | 68.29 (64.01 to 69.37) | 68.34 (64.76 to 71.28) | 67.02 (64.01 to 68.98) | +1.0% |
| Long object keys | 9.55 | 14.24 (13.39 to 15.20) | 14.39 (13.83 to 15.65) | 14.74 (13.66 to 16.65) | -0.7% |
| Broker 64 × 512 KiB | 63.52 | 51.88 (51.24 to 65.93) | 42.37 (37.00 to 50.62) | 54.72 (50.01 to 58.73) | -20.5% |
| Broker 512 × 16 KiB | 8.22 | 19.73 (18.14 to 23.07) | 18.42 (17.06 to 19.51) | 19.71 (19.00 to 20.91) | -6.6% |
| Broker 1,024 × 1 KiB | 1.18 | 10.94 (10.21 to 11.66) | 10.74 (10.48 to 11.32) | 10.96 (10.17 to 11.64) | -2.0% |

The elapsed times below cover the Mesh loop or collector only. Whole-process elapsed time and individual user and system CPU counters are in the archive. The Apple M5 Pro was shared with other active work. The raw rows record host load for each sample.

| Workload | A elapsed ms | Wide scan elapsed ms | B elapsed ms |
| --- | ---: | ---: | ---: |
| Plain Unicode | 76.84 (73.22 to 127.30) | 43.56 (39.26 to 57.14) | 76.27 (69.91 to 175.48) |
| Sparse escapes in Unicode | 80.15 (72.86 to 88.04) | 55.69 (51.27 to 97.94) | 80.59 (77.06 to 97.11) |
| Dense escape bursts | 65.09 (57.32 to 102.02) | 63.66 (50.19 to 221.42) | 61.32 (51.05 to 105.36) |
| Dense controls | 48.40 (44.09 to 67.92) | 62.61 (54.93 to 70.58) | 50.08 (46.68 to 60.16) |
| Dense controls after a plain prefix | 55.06 (49.33 to 70.15) | 66.55 (58.85 to 149.76) | 55.73 (45.60 to 59.48) |
| Small trees | 63.55 (59.80 to 77.18) | 61.19 (56.81 to 65.86) | 60.38 (56.73 to 80.66) |
| Long object keys | 7.56 (6.68 to 8.72) | 7.76 (6.90 to 8.50) | 7.54 (7.22 to 9.70) |
| Broker 64 × 512 KiB | 30.49 (26.17 to 34.25) | 20.45 (15.64 to 26.99) | 32.75 (29.34 to 45.04) |
| Broker 512 × 16 KiB | 9.55 (8.06 to 17.54) | 7.14 (6.53 to 9.55) | 9.21 (7.89 to 10.72) |
| Broker 1,024 × 1 KiB | 4.27 (3.72 to 4.70) | 4.03 (3.75 to 5.70) | 4.03 (3.56 to 4.87) |

Each ordinary Mesh run parses varied trees once and encodes each tree in every measured repetition. The program reads the encoded byte count through the original compiler's Bytes API. Full output is emitted and checked separately. A stock process sample found serde's byte-by-byte scan on the hot path alongside output allocation, copies, and garbage collection.

The broker collector uses the original application source and the frozen compiler. It retains the original batching and repeat encode behavior. The runtime was rebuilt at `dbcb6dbc1de3a6887fc3757c2b91f7974d74ed17` for matched controls and variants. The first rotation series includes the original pinned runtime. That series put large-batch CPU at 61.4 ms for the pinned runtime and 59.4 ms and 61.7 ms for the rebuilt controls. The application-level carry-once source was outside the experiment.

## Warm PostgreSQL producer

The original broker submitted the frozen 64 × 512 KiB batch to six disposable PostgreSQL 17.11 TLS clusters per arm. Each broker process was connected before the file submission. All runs verified 64 enqueues, 64 admissions, durable receipt IDs, row order, payload hashes, and the PostgreSQL commit timestamp for every row. Each batch carried 32 MiB of payload and the collector produced 66,607,436 spec bytes across 127 encodes.

| Arm | Broker CPU ms | Submit-to-commit ms |
| --- | ---: | ---: |
| A | 203.2 (193.8 to 228.4) | 723.1 (661.0 to 971.6) |
| 32-byte block scan | 189.2 (178.5 to 201.3) | 756.5 (457.5 to 865.0) |
| B | 206.5 (188.2 to 224.8) | 659.6 (518.8 to 869.0) |

The block scan reduced median broker process CPU by 7.6% against the duplicate controls. The clock ranges overlap. The 512- and 1,024-job cases passed stock PostgreSQL preflight, and their timed comparisons ran in the compiled collector. One preflight initially used the top-level transaction ID for the rows. Another counted the SQL enqueue function per collector chunk. Both failures are archived. The corrected driver checks the row commit timestamp, counts one enqueue SQL call per job, and counts one admission per chunk.

## Output and implementation checks

All timed variants passed byte-for-byte checks for ordinary trees and the frozen broker chunks. The checks cover duplicate keys and their position, nested arrays and objects, long keys, ASCII controls, quote and backslash escapes, multibyte text, signed and unsigned integer limits, negative zero, and exponent forms. A compiled C caller compared output for short raw strings and a string containing every byte. It also checked non-finite, subnormal, and extreme finite floats. The baseline and checked variants share SHA-256 `15b6eca931facc8f20a5902cb2c9fc46bdcc426bfbfc27297ddac4a7e1b99b54`.

The prototypes use serde's public byte-array formatter hook for ordinary string values. Serde still writes numbers, map keys, and individual escapes. The bounded encoder keeps the original writer, byte limit, and depth count. The main checkout has no runtime patch from the experiment. The prototype stopped before workspace and native product qualification.

The archive contains the eight source variants, all five timing series, the compiled probe sources, the verified output bytes, PostgreSQL receipts, and failure logs. The reservation variants add up to 128 bytes of headroom when a large fragment would otherwise fill a newly grown buffer. Reservation alone produced no consistent CPU gain. The 32-byte scan with a validated serde fallback incurred the cost of another UTF-8 pass on escaped Unicode. The final one-pass prefix variant reduced large collector CPU by 26% and increased dense-control CPU by 29%. [Evidence archive](evidence.tar.gz) · [Archive replay notes](REPLAY.md)
