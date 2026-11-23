# Native durable Jobs baseline

Mesh, Oban, River, and Apalis completed 25 four-engine blocks. The five profiles each ran in five rotated groups on CPUs 0 and 1 of the dedicated x86-64 Linode. The measured arms ran from 2026-11-23T16:53:22 UTC to 17:26:41 UTC on November 23, 2026. The series retained 100 measured arms and 60 full unmeasured warmups. All 35,040 measured jobs reached their native successful terminal state. The verifier matched every enqueue receipt and checked 70,080 distinct, ordered application commits.

The host used Ubuntu 24.04.5, ext4 on `/dev/sda`, and on-disk PostgreSQL 17.11 over verified TLS. All profiles enabled durable commit settings and used the frozen inputs and physical connection caps. Mesh was GNU/Linux AMD64 port `3f69b150`. Oban was Basic 2.24.2 with OTP 29.1.1, Elixir 1.20.4, and the pinned resolver and scheduler flags. River was v0.51.2 with pgx v5.13.0 and Go 1.26.4. Apalis and `apalis-postgres` were both 1.2.0. Maximum measured steal was 0.05% and maximum I/O wait was 2.76% in the sampled submit windows. [Release pins](inventory.json), [validated block index](index.json), [reversed controls](../2026-11-23-controls/README.md)

## Submit to durable settlement

Jobs/s. Cells show the median [minimum–maximum] of five measured arms. The window includes batch input and submission before the first enqueue commit.

| Profile | Mesh | Oban | River | Apalis |
| --- | ---: | ---: | ---: | ---: |
| Dense no-op | 170.90 [168.82–175.91] | 636.45 [630.17–654.84] | 869.81 [849.63–886.74] | 1,138.66 [1,104.20–1,156.43] |
| Slow | 163.17 [150.08–166.41] | 246.14 [231.96–250.05] | 286.60 [252.79–295.22] | 363.33 [302.87–383.96] |
| Sparse | 2.65 [2.64–2.65] | 2.03 [2.03–2.04] | 2.08 [2.08–2.08] | 2.50 [2.49–2.50] |
| Short | 198.92 [174.12–209.21] | 273.86 [249.71–308.78] | 726.67 [466.52–739.69] | 818.93 [317.35–897.81] |
| Large payload | 23.81 [23.00–24.02] | 24.76 [23.84–24.82] | 19.99 [19.76–21.06] | 38.64 [2.39–40.35] |

## Durable enqueue commit to durable settlement

Jobs/s. The window starts at the first confirmed producer commit and ends at the final terminal commit. Sparse includes the fixed spacing between arrivals in both windows.

| Profile | Mesh | Oban | River | Apalis |
| --- | ---: | ---: | ---: | ---: |
| Dense no-op | 190.34 [187.65–194.81] | 707.60 [698.12–719.11] | 926.42 [918.08–954.11] | 1,187.19 [1,148.53–1,217.25] |
| Slow | 205.63 [184.72–210.34] | 323.96 [301.80–334.50] | 390.61 [333.41–405.82] | 471.64 [377.49–510.64] |
| Sparse | 2.65 [2.64–2.66] | 2.04 [2.03–2.04] | 2.08 [2.08–2.08] | 2.50 [2.49–2.50] |
| Short | 224.91 [198.92–235.23] | 303.47 [278.79–349.47] | 791.18 [493.74–807.88] | 876.36 [327.23–993.88] |
| Large payload | 75.69 [75.02–79.48] | 124.63 [119.69–127.84] | 68.16 [66.91–76.10] | 362.69 [2.53–391.47] |

## Where Mesh spends the dense window

Dense no-op is Mesh's largest gap against Oban. Mesh's median first claim was 7.93 ms after commit versus 28.12 ms for Oban. The median interval from first claim to last settlement was 5.38 seconds for Mesh versus 1.41 seconds for Oban. Mesh's median claim-to-handler stage was 66.67 ms versus 20.22 ms, and its effect-to-settlement stage was 38.39 ms versus 7.34 ms. Mesh's two pinned CPUs were busy for a median 93.76% of the submit window, with 39.40% in kernel time and 0.08% in I/O wait. The stage medians are computed independently. [Stage and host evidence](stages.json)

The Apalis large range includes a reconciled slow arm. The Rust and Go short ranges also vary. The archive retains every validated arm. The CSV includes both throughput windows, pickup and settlement p99s, runner CPU, RSS, connection peaks, CPU steal, and I/O wait. [Comparison CSV](comparison.csv)

```sh
python3 -B tools/durable-jobs-bench/run.py summarize --series 2026-11-23-baseline
python3 -B tools/durable-jobs-bench/localize.py tools/durable-jobs-bench/results/2026-11-23-baseline
```
