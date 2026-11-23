# Native four-engine due-query repeat

The candidate repeated all five profiles in five rotated groups. All 100 measured arms and 60 unmeasured warmups passed. The measured arms reconciled 35,040 jobs and 70,080 ordered application commits. The four engines reused the baseline's host boot, CPU mask, disk, PostgreSQL 17.11 TLS setup, and frozen inputs. The candidate changed the Mesh due query and rebuilt its Rust runtime. Oban, River, and Apalis used their baseline release and wrapper fingerprints. [Build and plan witness](../2026-11-23-mesh-due-range/README.md), [validated blocks](index.json)

The table shows submit-to-durable-settlement jobs/s. Cells show the median [minimum–maximum] across five arms. The two full series ran at different times. The alternating no-op pairs test the old/new Mesh change directly.

| Profile | Baseline Mesh | Candidate Mesh | Oban | River | Apalis |
| --- | ---: | ---: | ---: | ---: | ---: |
| Dense no-op | 170.90 [168.82–175.91] | 175.36 [173.13–178.39] | 642.54 [627.00–664.75] | 829.92 [826.66–881.64] | 1,121.51 [1,109.84–1,149.12] |
| Slow | 163.17 [150.08–166.41] | 161.16 [154.38–163.59] | 242.59 [236.09–246.96] | 253.08 [252.27–295.38] | 347.54 [331.66–379.48] |
| Sparse | 2.65 [2.64–2.65] | 2.65 [2.63–2.66] | 2.03 [2.03–2.04] | 2.08 [2.08–2.08] | 2.49 [2.46–2.50] |
| Short | 198.92 [174.12–209.21] | 198.62 [137.74–208.56] | 272.20 [265.61–321.34] | 563.51 [470.45–717.92] | 881.47 [843.40–932.85] |
| Large payload | 23.81 [23.00–24.02] | 24.01 [23.57–24.37] | 24.83 [24.41–25.15] | 20.44 [19.63–20.78] | 39.13 [36.44–39.68] |

The largest sampled steal was 0.57%. The largest I/O wait was 5.43%. Apalis large finished at 36.44–39.68 jobs/s across the five repeat arms. Its isolated 25-second terminal delay remains in the baseline archive. [Both measured windows and resource metrics](comparison.csv), [original baseline](../2026-11-23-baseline/README.md)

```sh
python3 -B tools/durable-jobs-bench/run.py summarize --series 2026-11-23-due-range-repeat
```
