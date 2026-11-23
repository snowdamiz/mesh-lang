# 2026-11-23-baseline stage analysis

Scoring phase `measured` · 25/25 selected blocks · 25 planned in the series.

Replay with `python3 -B tools/durable-jobs-bench/analyze_stages.py --series 2026-11-23-baseline`. To publish a complete series, replay without selectors and append `--publish`.

[Per-arm evidence and sources](analysis.json) · SHA-256 `09a53a429bda37745a07516601c35b44c6f81428e1afd44c33e87abd8b368049`.

## noop

| Arm metric | Mesh | Oban | Go (River) | Rust (Apalis) |
| --- | ---: | ---: | ---: | ---: |
| commit jobs/s | 190.338 | 707.600 | 926.421 | 1187.193 |
| submit jobs/s | 170.903 | 636.449 | 869.815 | 1138.656 |
| middle80 jobs/s | 199.152 | 756.291 | 2388.943 | 1251.559 |
| First submit → first commit (ms) | 573.046 | 158.170 | 73.561 | 39.378 |
| First commit → first claim (ms) | 7.933 | 28.123 | 2.509 | — |
| First → last claim (ms) | 5287.000 | 1365.291 | 854.740 | — |
| First claim → last terminal (ms) | 5376.376 | 1414.032 | 1102.676 | — |
| Commit → pickup p99 (ms) | 5310.721 | 1408.183 | 859.576 | 840.106 |
| Submit → pickup p99 (ms) | 5914.067 | 1575.711 | 930.913 | 876.873 |
| Runner CPU per job (ms) | 5.805 | 9.962 | 0.764 | 0.592 |
| Terminal transactions | 1024.000 | 1024.000 | 34.000 | 179.000 |
| Largest terminal transaction (jobs) | 1.000 | 1.000 | 74.000 | 8.000 |
| Producer commit → recorded queue claim mean (ms) | 2672.713 | 748.952 | 553.832 | — |
| Recorded queue claim → handler entry mean (ms) | 70.414 | 28.103 | 11.317 | — |
| Handler entry → first application transaction commit mean (ms) | 3.802 | 6.805 | 12.719 | 3.940 |
| First → second application transaction commit mean (ms) | 12.514 | 11.401 | 7.706 | 5.790 |
| Second application commit → durable terminal commit mean (ms) | 43.327 | 9.278 | 153.957 | 21.533 |
| entry to terminal sum (job-ms) | 61134.756 | 28180.961 | 177404.135 | 32516.723 |
| claim to terminal sum (job-ms) | 133238.284 | 56782.337 | 190265.722 | — |

| Peer | Window | Mesh:peer ratio median [min, max] | Mesh wins | Peer wins | Ties | Undefined | Mesh − peer ms median [min, max] |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| oban | commit | 0.265813 [0.263069, 0.279051] | 0 | 5 | 0 | 0 | 3933.102 [3789.583, 4009.804] |
| oban | submit | 0.265248 [0.263639, 0.279153] | 0 | 5 | 0 | 0 | 4384.110 [4196.095, 4456.808] |
| oban | middle80 | 0.263802 [0.260467, 0.268418] | 0 | 5 | 0 | 0 | 3034.039 [2961.893, 3080.781] |
| go | commit | 0.204180 [0.200484, 0.207322] | 0 | 5 | 0 | 0 | 4264.531 [4183.132, 4344.613] |
| go | submit | 0.198382 [0.194011, 0.198695] | 0 | 5 | 0 | 0 | 4819.556 [4666.272, 4860.503] |
| go | middle80 | 0.084782 [0.078993, 0.112886] | 0 | 5 | 0 | 0 | 3731.573 [3646.079, 3798.778] |
| rust | commit | 0.160326 [0.154534, 0.166429] | 0 | 5 | 0 | 0 | 4517.367 [4384.413, 4613.664] |
| rust | submit | 0.149728 [0.145982, 0.156560] | 0 | 5 | 0 | 0 | 5104.291 [4909.717, 5180.248] |
| rust | middle80 | 0.158848 [0.148352, 0.166773] | 0 | 5 | 0 | 0 | 3462.057 [3393.118, 3570.521] |

| Matched residence sum | Wall window | Opposite | Same | Tied |
| --- | --- | ---: | ---: | ---: |
| Mesh vs oban, entry to terminal | commit window | 0 | 5 | 0 |
| Mesh vs oban, claim to terminal | commit window | 0 | 5 | 0 |
| Mesh vs oban, entry to terminal | post claim drain | 0 | 5 | 0 |
| Mesh vs oban, claim to terminal | post claim drain | 0 | 5 | 0 |
| Mesh vs go, entry to terminal | commit window | 5 | 0 | 0 |
| Mesh vs go, claim to terminal | commit window | 5 | 0 | 0 |
| Mesh vs go, entry to terminal | post claim drain | 5 | 0 | 0 |
| Mesh vs go, claim to terminal | post claim drain | 5 | 0 | 0 |
| Mesh vs rust, entry to terminal | commit window | 0 | 5 | 0 |

| Mesh segment set to zero | Max saving, median [min, max] ms | Peer | Commit gaps covered / bounded / slower | Submit gaps covered / bounded / slower |
| --- | ---: | --- | ---: | ---: |
| Producer commit → recorded queue claim | 5114.133 [4978.675, 5184.922] | oban | 5/5/5 | 5/5/5 |
| Producer commit → recorded queue claim | 5114.133 [4978.675, 5184.922] | go | 5/5/5 | 5/5/5 |
| Producer commit → recorded queue claim | 5114.133 [4978.675, 5184.922] | rust | 5/5/5 | 5/5/5 |
| Recorded queue claim → handler entry | 49.747 [41.698, 64.055] | oban | 0/5/5 | 0/5/5 |
| Recorded queue claim → handler entry | 49.747 [41.698, 64.055] | go | 0/5/5 | 0/5/5 |
| Recorded queue claim → handler entry | 49.747 [41.698, 64.055] | rust | 0/5/5 | 0/5/5 |
| Handler entry → first application transaction commit | 1.298 [0.802, 4.002] | oban | 0/5/5 | 0/5/5 |
| Handler entry → first application transaction commit | 1.298 [0.802, 4.002] | go | 0/5/5 | 0/5/5 |
| Handler entry → first application transaction commit | 1.298 [0.802, 4.002] | rust | 0/5/5 | 0/5/5 |
| Second application commit → durable terminal commit | 10.306 [5.207, 19.639] | oban | 0/5/5 | 0/5/5 |
| Second application commit → durable terminal commit | 10.306 [5.207, 19.639] | go | 0/5/5 | 0/5/5 |
| Second application commit → durable terminal commit | 10.306 [5.207, 19.639] | rust | 0/5/5 | 0/5/5 |

## slow

| Arm metric | Mesh | Oban | Go (River) | Rust (Apalis) |
| --- | ---: | ---: | ---: | ---: |
| commit jobs/s | 205.634 | 323.955 | 390.611 | 471.640 |
| submit jobs/s | 163.172 | 246.142 | 286.596 | 363.325 |
| middle80 jobs/s | 216.203 | 435.436 | 588.027 | 486.403 |
| First submit → first commit (ms) | 642.558 | 499.632 | 475.722 | 334.164 |
| First commit → first claim (ms) | 5.230 | 262.755 | 24.310 | — |
| First → last claim (ms) | 2301.000 | 1198.066 | 1160.941 | — |
| First claim → last terminal (ms) | 2477.344 | 1317.710 | 1286.659 | — |
| Commit → pickup p99 (ms) | 2357.542 | 1470.268 | 1189.457 | 969.793 |
| Submit → pickup p99 (ms) | 3007.917 | 1967.238 | 1662.455 | 1293.560 |
| Runner CPU per job (ms) | 4.741 | 3.950 | 0.895 | 0.479 |
| Terminal transactions | 512.000 | 512.000 | 7.000 | 24.000 |
| Largest terminal transaction (jobs) | 1.000 | 1.000 | 124.000 | 63.000 |
| Producer commit → recorded queue claim mean (ms) | 1181.371 | 894.935 | 674.307 | — |
| Recorded queue claim → handler entry mean (ms) | 44.979 | 14.280 | 8.630 | — |
| Handler entry → first application transaction commit mean (ms) | 1.403 | 7.015 | 28.869 | 7.274 |
| First → second application transaction commit mean (ms) | 104.122 | 110.355 | 104.169 | 106.632 |
| Second application commit → durable terminal commit mean (ms) | 30.022 | 10.499 | 116.761 | 19.066 |
| entry to terminal sum (job-ms) | 69360.655 | 64884.711 | 127823.367 | 68285.385 |
| claim to terminal sum (job-ms) | 92390.073 | 72196.243 | 131687.015 | — |

| Peer | Window | Mesh:peer ratio median [min, max] | Mesh wins | Peer wins | Ties | Undefined | Mesh − peer ms median [min, max] |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| oban | commit | 0.643048 [0.552234, 0.681367] | 0 | 5 | 0 | 0 | 877.184 [793.350, 1241.072] |
| oban | submit | 0.654860 [0.600195, 0.704955] | 0 | 5 | 0 | 0 | 1082.977 [923.826, 1363.946] |
| oban | middle80 | 0.487553 [0.425261, 0.595961] | 0 | 5 | 0 | 0 | 971.790 [755.084, 1255.824] |
| go | commit | 0.533390 [0.490323, 0.630891] | 0 | 5 | 0 | 0 | 1189.438 [898.457, 1311.439] |
| go | submit | 0.569346 [0.537869, 0.658313] | 0 | 5 | 0 | 0 | 1357.718 [1051.261, 1490.091] |
| go | middle80 | 0.370420 [0.319101, 0.392171] | 0 | 5 | 0 | 0 | 1176.583 [1142.552, 1487.787] |
| rust | commit | 0.435998 [0.387421, 0.551929] | 0 | 5 | 0 | 0 | 1404.283 [1101.103, 1697.883] |
| rust | submit | 0.449645 [0.413071, 0.538752] | 0 | 5 | 0 | 0 | 1723.235 [1447.300, 2002.326] |
| rust | middle80 | 0.451041 [0.383577, 0.603790] | 0 | 5 | 0 | 0 | 1025.916 [751.361, 1346.905] |

| Matched residence sum | Wall window | Opposite | Same | Tied |
| --- | --- | ---: | ---: | ---: |
| Mesh vs oban, entry to terminal | commit window | 1 | 4 | 0 |
| Mesh vs oban, claim to terminal | commit window | 0 | 5 | 0 |
| Mesh vs oban, entry to terminal | post claim drain | 1 | 4 | 0 |
| Mesh vs oban, claim to terminal | post claim drain | 0 | 5 | 0 |
| Mesh vs go, entry to terminal | commit window | 5 | 0 | 0 |
| Mesh vs go, claim to terminal | commit window | 5 | 0 | 0 |
| Mesh vs go, entry to terminal | post claim drain | 5 | 0 | 0 |
| Mesh vs go, claim to terminal | post claim drain | 5 | 0 | 0 |
| Mesh vs rust, entry to terminal | commit window | 2 | 3 | 0 |

| Mesh segment set to zero | Max saving, median [min, max] ms | Peer | Commit gaps covered / bounded / slower | Submit gaps covered / bounded / slower |
| --- | ---: | --- | ---: | ---: |
| Producer commit → recorded queue claim | 2257.303 [2189.677, 2498.205] | oban | 5/5/5 | 5/5/5 |
| Producer commit → recorded queue claim | 2257.303 [2189.677, 2498.205] | go | 5/5/5 | 5/5/5 |
| Producer commit → recorded queue claim | 2257.303 [2189.677, 2498.205] | rust | 5/5/5 | 5/5/5 |
| Recorded queue claim → handler entry | 65.871 [57.468, 80.526] | oban | 0/5/5 | 0/5/5 |
| Recorded queue claim → handler entry | 65.871 [57.468, 80.526] | go | 0/5/5 | 0/5/5 |
| Recorded queue claim → handler entry | 65.871 [57.468, 80.526] | rust | 0/5/5 | 0/5/5 |
| Handler entry → first application transaction commit | 1.060 [0.739, 1.475] | oban | 0/5/5 | 0/5/5 |
| Handler entry → first application transaction commit | 1.060 [0.739, 1.475] | go | 0/5/5 | 0/5/5 |
| Handler entry → first application transaction commit | 1.060 [0.739, 1.475] | rust | 0/5/5 | 0/5/5 |
| Second application commit → durable terminal commit | 3.230 [2.592, 6.010] | oban | 0/5/5 | 0/5/5 |
| Second application commit → durable terminal commit | 3.230 [2.592, 6.010] | go | 0/5/5 | 0/5/5 |
| Second application commit → durable terminal commit | 3.230 [2.592, 6.010] | rust | 0/5/5 | 0/5/5 |

## sparse

| Arm metric | Mesh | Oban | Go (River) | Rust (Apalis) |
| --- | ---: | ---: | ---: | ---: |
| commit jobs/s | 2.648 | 2.039 | 2.084 | 2.497 |
| submit jobs/s | 2.646 | 2.034 | 2.084 | 2.495 |
| middle80 jobs/s | 2.544 | 1.984 | 2.000 | 2.395 |
| First submit → first commit (ms) | 6.762 | 32.681 | 3.999 | 6.131 |
| First commit → first claim (ms) | 3.454 | 168.052 | 0.906 | — |
| First → last claim (ms) | 9049.000 | 11592.048 | 11286.715 | — |
| First claim → last terminal (ms) | 9060.821 | 11596.926 | 11513.760 | — |
| Commit → pickup p99 (ms) | 14.443 | 226.004 | 4.530 | 5.719 |
| Submit → pickup p99 (ms) | 19.936 | 228.399 | 6.492 | 9.007 |
| Runner CPU per job (ms) | 18.383 | 55.948 | 6.962 | 3.542 |
| Terminal transactions | 24.000 | 24.000 | 24.000 | 24.000 |
| Largest terminal transaction (jobs) | 1.000 | 1.000 | 1.000 | 1.000 |
| Producer commit → recorded queue claim mean (ms) | 3.649 | 166.128 | 0.956 | — |
| Recorded queue claim → handler entry mean (ms) | 6.257 | 2.200 | 0.897 | — |
| Handler entry → first application transaction commit mean (ms) | 1.434 | 0.541 | 0.180 | 0.885 |
| First → second application transaction commit mean (ms) | 2.409 | 2.378 | 1.554 | 2.644 |
| Second application commit → durable terminal commit mean (ms) | 5.011 | 0.738 | 184.455 | 1.233 |
| entry to terminal sum (job-ms) | 216.821 | 86.885 | 4475.825 | 113.045 |
| claim to terminal sum (job-ms) | 369.858 | 137.966 | 4495.812 | — |
| Settled → next submit gaps (ms) | 8521.461 | 7604.258 | 6959.911 | 9390.217 |
| Active submit → terminal time (ms) | 552.380 | 4221.450 | 4559.009 | 233.058 |

| Peer | Window | Mesh:peer ratio median [min, max] | Mesh wins | Peer wins | Ties | Undefined | Mesh − peer ms median [min, max] |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| oban | commit | 1.301910 [1.292147, 1.303498] | 5 | 0 | 0 | 0 | -2729.248 [-2745.009, -2656.149] |
| oban | submit | 1.304684 [1.294747, 1.306287] | 5 | 0 | 0 | 0 | -2755.791 [-2772.791, -2681.705] |
| oban | middle80 | 1.281813 [1.276510, 1.286520] | 5 | 0 | 0 | 0 | -2217.702 [-2245.104, -2184.120] |
| go | commit | 1.270318 [1.266494, 1.273971] | 5 | 0 | 0 | 0 | -2450.289 [-2476.316, -2422.909] |
| go | submit | 1.269886 [1.266016, 1.273549] | 5 | 0 | 0 | 0 | -2448.191 [-2474.177, -2420.299] |
| go | middle80 | 1.271727 [1.265816, 1.276183] | 5 | 0 | 0 | 0 | -2136.626 [-2164.104, -2099.650] |
| rust | commit | 1.062064 [1.056415, 1.063567] | 5 | 0 | 0 | 0 | -562.578 [-574.556, -512.918] |
| rust | submit | 1.061942 [1.056377, 1.063482] | 5 | 0 | 0 | 0 | -561.888 [-574.180, -512.938] |
| rust | middle80 | 1.062972 [1.056339, 1.065106] | 5 | 0 | 0 | 0 | -495.553 [-510.672, -445.015] |

| Matched residence sum | Wall window | Opposite | Same | Tied |
| --- | --- | ---: | ---: | ---: |
| Mesh vs oban, entry to terminal | commit window | 5 | 0 | 0 |
| Mesh vs oban, claim to terminal | commit window | 5 | 0 | 0 |
| Mesh vs oban, entry to terminal | post claim drain | 5 | 0 | 0 |
| Mesh vs oban, claim to terminal | post claim drain | 5 | 0 | 0 |
| Mesh vs go, entry to terminal | commit window | 0 | 5 | 0 |
| Mesh vs go, claim to terminal | commit window | 0 | 5 | 0 |
| Mesh vs go, entry to terminal | post claim drain | 0 | 5 | 0 |
| Mesh vs go, claim to terminal | post claim drain | 0 | 5 | 0 |
| Mesh vs rust, entry to terminal | commit window | 5 | 0 | 0 |

## short

| Arm metric | Mesh | Oban | Go (River) | Rust (Apalis) |
| --- | ---: | ---: | ---: | ---: |
| commit jobs/s | 224.906 | 303.470 | 791.178 | 876.358 |
| submit jobs/s | 198.923 | 273.860 | 726.666 | 818.927 |
| middle80 jobs/s | 226.572 | 730.496 | — | 913.290 |
| First submit → first commit (ms) | 68.674 | 48.263 | 14.363 | 10.446 |
| First commit → first claim (ms) | 3.483 | 222.468 | 1.131 | — |
| First → last claim (ms) | 549.000 | 192.730 | 147.353 | — |
| First claim → last terminal (ms) | 565.710 | 201.678 | 160.620 | — |
| Commit → pickup p99 (ms) | 538.425 | 412.005 | 150.684 | 138.281 |
| Submit → pickup p99 (ms) | 613.500 | 457.609 | 163.848 | 148.524 |
| Runner CPU per job (ms) | 4.690 | 11.080 | 0.832 | 0.630 |
| Terminal transactions | 128.000 | 128.000 | 1.000 | 16.000 |
| Largest terminal transaction (jobs) | 1.000 | 1.000 | 128.000 | 15.000 |
| Producer commit → recorded queue claim mean (ms) | 256.073 | 329.321 | 91.778 | — |
| Recorded queue claim → handler entry mean (ms) | 27.684 | 6.285 | 2.511 | — |
| Handler entry → first application transaction commit mean (ms) | 1.402 | 2.956 | 7.140 | 2.018 |
| First → second application transaction commit mean (ms) | 4.269 | 4.897 | 3.346 | 4.380 |
| Second application commit → durable terminal commit mean (ms) | 25.243 | 3.550 | 57.192 | 4.984 |
| entry to terminal sum (job-ms) | 3918.955 | 1454.637 | 8623.140 | 1431.109 |
| claim to terminal sum (job-ms) | 7469.515 | 2259.083 | 8960.742 | — |

| Peer | Window | Mesh:peer ratio median [min, max] | Mesh wins | Peer wins | Ties | Undefined | Mesh − peer ms median [min, max] |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| oban | commit | 0.691874 [0.644366, 0.806725] | 0 | 5 | 0 | 0 | 177.294 [108.191, 221.693] |
| oban | submit | 0.692856 [0.635811, 0.796961] | 0 | 5 | 0 | 0 | 197.637 [124.225, 267.719] |
| oban | middle80 | 0.311550 [0.273467, 0.374018] | 0 | 5 | 0 | 0 | 316.517 [267.872, 377.351] |
| go | commit | 0.297313 [0.278388, 0.452067] | 0 | 5 | 0 | 0 | 384.235 [311.455, 415.211] |
| go | submit | 0.287903 [0.268621, 0.430660] | 0 | 5 | 0 | 0 | 462.416 [362.722, 471.155] |
| go | middle80 | 0.205135 [0.205135, 0.205135] | 0 | 1 | 0 | 4 | 364.855 [364.855, 364.855] |
| rust | commit | 0.256957 [0.200143, 0.687298] | 0 | 5 | 0 | 0 | 422.359 [177.967, 514.693] |
| rust | submit | 0.245337 [0.193942, 0.626101] | 0 | 5 | 0 | 0 | 480.352 [240.866, 592.542] |
| rust | middle80 | 0.250483 [0.183008, 0.737422] | 0 | 5 | 0 | 0 | 340.743 [121.243, 424.334] |

| Matched residence sum | Wall window | Opposite | Same | Tied |
| --- | --- | ---: | ---: | ---: |
| Mesh vs oban, entry to terminal | commit window | 0 | 5 | 0 |
| Mesh vs oban, claim to terminal | commit window | 0 | 5 | 0 |
| Mesh vs oban, entry to terminal | post claim drain | 0 | 5 | 0 |
| Mesh vs oban, claim to terminal | post claim drain | 0 | 5 | 0 |
| Mesh vs go, entry to terminal | commit window | 5 | 0 | 0 |
| Mesh vs go, claim to terminal | commit window | 5 | 0 | 0 |
| Mesh vs go, entry to terminal | post claim drain | 5 | 0 | 0 |
| Mesh vs go, claim to terminal | post claim drain | 5 | 0 | 0 |
| Mesh vs rust, entry to terminal | commit window | 1 | 4 | 0 |

| Mesh segment set to zero | Max saving, median [min, max] ms | Peer | Commit gaps covered / bounded / slower | Submit gaps covered / bounded / slower |
| --- | ---: | --- | ---: | ---: |
| Producer commit → recorded queue claim | 491.366 [470.391, 533.610] | oban | 5/5/5 | 5/5/5 |
| Producer commit → recorded queue claim | 491.366 [470.391, 533.610] | go | 5/5/5 | 5/5/5 |
| Producer commit → recorded queue claim | 491.366 [470.391, 533.610] | rust | 5/5/5 | 4/5/5 |
| Recorded queue claim → handler entry | 7.972 [3.423, 17.941] | oban | 0/5/5 | 0/5/5 |
| Recorded queue claim → handler entry | 7.972 [3.423, 17.941] | go | 0/5/5 | 0/5/5 |
| Recorded queue claim → handler entry | 7.972 [3.423, 17.941] | rust | 0/5/5 | 0/5/5 |
| Handler entry → first application transaction commit | 0.727 [0.593, 1.518] | oban | 0/5/5 | 0/5/5 |
| Handler entry → first application transaction commit | 0.727 [0.593, 1.518] | go | 0/5/5 | 0/5/5 |
| Handler entry → first application transaction commit | 0.727 [0.593, 1.518] | rust | 0/5/5 | 0/5/5 |
| Second application commit → durable terminal commit | 8.863 [8.209, 9.498] | oban | 0/5/5 | 0/5/5 |
| Second application commit → durable terminal commit | 8.863 [8.209, 9.498] | go | 0/5/5 | 0/5/5 |
| Second application commit → durable terminal commit | 8.863 [8.209, 9.498] | rust | 0/5/5 | 0/5/5 |

## large

| Arm metric | Mesh | Oban | Go (River) | Rust (Apalis) |
| --- | ---: | ---: | ---: | ---: |
| commit jobs/s | 75.694 | 124.631 | 68.159 | 362.693 |
| submit jobs/s | 23.809 | 24.762 | 19.987 | 38.642 |
| middle80 jobs/s | 82.464 | 137.434 | 196.623 | 741.343 |
| First submit → first commit (ms) | 1859.551 | 2082.863 | 2258.614 | 1464.272 |
| First commit → first claim (ms) | 28.221 | 22.201 | 0.806 | — |
| First → last claim (ms) | 727.000 | 245.346 | 16.815 | — |
| First claim → last terminal (ms) | 815.568 | 487.378 | 938.054 | — |
| Commit → pickup p99 (ms) | 831.166 | 478.472 | 597.553 | 132.738 |
| Submit → pickup p99 (ms) | 2675.006 | 2556.780 | 2856.167 | 1591.250 |
| Runner CPU per job (ms) | 25.801 | 147.031 | 14.839 | 5.906 |
| Terminal transactions | 64.000 | 64.000 | 12.000 | 18.000 |
| Largest terminal transaction (jobs) | 1.000 | 1.000 | 8.000 | 7.000 |
| Producer commit → recorded queue claim mean (ms) | 406.757 | 159.066 | 9.092 | — |
| Recorded queue claim → handler entry mean (ms) | 76.289 | 190.437 | 374.449 | — |
| Handler entry → first application transaction commit mean (ms) | 6.629 | 7.599 | 99.590 | 16.010 |
| First → second application transaction commit mean (ms) | 8.539 | 10.124 | 20.011 | 12.781 |
| Second application commit → durable terminal commit mean (ms) | 19.247 | 11.925 | 240.639 | 19.539 |
| entry to terminal sum (job-ms) | 2199.851 | 1845.488 | 22486.104 | 3070.989 |
| claim to terminal sum (job-ms) | 7259.999 | 14023.675 | 46160.954 | — |

| Peer | Window | Mesh:peer ratio median [min, max] | Mesh wins | Peer wins | Ties | Undefined | Mesh − peer ms median [min, max] |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| oban | commit | 0.627023 [0.586803, 0.657633] | 0 | 5 | 0 | 0 | 315.355 [278.378, 352.510] |
| oban | submit | 0.962631 [0.937709, 1.000142] | 1 | 4 | 0 | 0 | 103.424 [-0.381, 171.314] |
| oban | middle80 | 0.591189 [0.580648, 0.674214] | 0 | 5 | 0 | 0 | 261.564 [195.885, 270.909] |
| go | commit | 1.110545 [1.034273, 1.187877] | 5 | 0 | 0 | 0 | -93.467 [-151.279, -27.867] |
| go | submit | 1.191226 [1.104835, 1.215746] | 5 | 0 | 0 | 0 | -514.025 [-574.910, -288.321] |
| go | middle80 | 0.413225 [0.263638, 0.936250] | 0 | 5 | 0 | 0 | 375.538 [37.744, 475.703] |
| rust | commit | 0.215378 [0.193360, 29.910768] | 1 | 4 | 0 | 0 | 631.780 [-24491.179, 682.021] |
| rust | submit | 0.597944 [0.591072, 9.961133] | 1 | 4 | 0 | 0 | 1097.441 [-24087.910, 1118.525] |
| rust | middle80 | 0.108577 [0.042429, 0.139067] | 0 | 5 | 0 | 0 | 551.765 [511.924, 612.848] |

| Matched residence sum | Wall window | Opposite | Same | Tied |
| --- | --- | ---: | ---: | ---: |
| Mesh vs oban, entry to terminal | commit window | 1 | 4 | 0 |
| Mesh vs oban, claim to terminal | commit window | 5 | 0 | 0 |
| Mesh vs oban, entry to terminal | post claim drain | 1 | 4 | 0 |
| Mesh vs oban, claim to terminal | post claim drain | 5 | 0 | 0 |
| Mesh vs go, entry to terminal | commit window | 0 | 5 | 0 |
| Mesh vs go, claim to terminal | commit window | 0 | 5 | 0 |
| Mesh vs go, entry to terminal | post claim drain | 0 | 5 | 0 |
| Mesh vs go, claim to terminal | post claim drain | 0 | 5 | 0 |
| Mesh vs rust, entry to terminal | commit window | 4 | 1 | 0 |

| Mesh segment set to zero | Max saving, median [min, max] ms | Peer | Commit gaps covered / bounded / slower | Submit gaps covered / bounded / slower |
| --- | ---: | --- | ---: | ---: |
| Producer commit → recorded queue claim | 650.744 [609.214, 707.493] | oban | 5/5/5 | 4/4/4 |
| Producer commit → recorded queue claim | 650.744 [609.214, 707.493] | go | 0/0/0 | 0/0/0 |
| Producer commit → recorded queue claim | 650.744 [609.214, 707.493] | rust | 1/4/4 | 0/4/4 |
| Recorded queue claim → handler entry | 57.728 [28.541, 72.908] | oban | 0/5/5 | 0/4/4 |
| Recorded queue claim → handler entry | 57.728 [28.541, 72.908] | go | 0/0/0 | 0/0/0 |
| Recorded queue claim → handler entry | 57.728 [28.541, 72.908] | rust | 0/4/4 | 0/4/4 |
| Handler entry → first application transaction commit | 3.019 [2.570, 7.988] | oban | 0/5/5 | 0/4/4 |
| Handler entry → first application transaction commit | 3.019 [2.570, 7.988] | go | 0/0/0 | 0/0/0 |
| Handler entry → first application transaction commit | 3.019 [2.570, 7.988] | rust | 0/4/4 | 0/4/4 |
| Second application commit → durable terminal commit | 5.576 [4.126, 11.127] | oban | 0/5/5 | 0/4/4 |
| Second application commit → durable terminal commit | 5.576 [4.126, 11.127] | go | 0/0/0 | 0/0/0 |
| Second application commit → durable terminal commit | 5.576 [4.126, 11.127] | rust | 0/4/4 | 0/4/4 |

### Apalis large settlement

Observed rates across 5 measured Apalis large ledgers.

| Window | Min jobs/s | Median jobs/s | Max jobs/s |
| --- | ---: | ---: | ---: |
| Enqueue commit to settlement | 2.53 | 362.69 | 391.47 |
| Submit to settlement | 2.39 | 38.64 | 40.35 |

In group 02, 63 of 64 jobs had settled before the final terminal timestamp. The penultimate job settled at 177.952 ms after the enqueue commit.

The last job to settle, `job_000034`, entered at 111.079 ms, committed its first application write at 137.503 ms, and committed its second at 166.438 ms. Its terminal commit landed at 25,338.309 ms, a 25,171.871 ms settlement interval. All offsets start at the enqueue commit. The job had one expected attempt (`fail_first=false`, `attempt=1`, `attempts=1`, `effects=1`). [Group 02 archive, `adapter/ledger.json`](../2026-11-23-baseline/blocks/02-large.1795453397938584000/measured-rust/raw.tar.gz)

The 121 host samples bracketing the submit-to-terminal window span 27,088.605 ms. The pinned CPUs (0, 1) recorded 0.000% steal and 0.018% I/O wait. The ledger accounts for 368.733 ms of runner CPU across the arm. The PostgreSQL mount device recorded 41 ms busy and 60 flushes across those samples. [Group 02 archive, `host-samples.jsonl`](../2026-11-23-baseline/blocks/02-large.1795453397938584000/measured-rust/raw.tar.gz)

### Reading the tables

- Full commit and submit rates use the same end, the latest durable terminal
  commit. The starts are the earliest producer commit and earliest submit
  respectively. Matched River arms start submit before reading and decoding
  the batch. The original River receipt began after decoding and is SETUP only.
- Middle 80% uses terminal ranks floor(n/10) and n − floor(n/10), with a
  minimum starting rank of one. It divides the rank difference by the elapsed
  time, e.g. 820 intervals from ranks 102 to 922 for the 1,024-job stock
  comparison. Tied completions stay in the ranked sample. The companion JSON
  records how many jobs share each boundary timestamp. A zero span is
  undefined. The full and middle windows use the same selected arms.
- Pickup is durable enqueue commit or submit to handler entry, with the
  matrix's nearest-rank p99 and zero floor. Phase means use signed differences
  of stored timestamps. Reversals within the matrix tolerance and exact ties
  are counted in the JSON. Mesh claim is stored in floored milliseconds. Per-job
  differences near that resolution cannot distinguish order or real savings.
- The claim stamp records the queue's claim event. It can precede claim
  transaction completion. Apalis retains a last-lock field with different
  semantics and has no verified claim stamp. Its three common application and
  terminal segments and entry-to-terminal residence remain comparable.
- Commit → claim includes backlog residence. The first-claim delay and
  the span of claims distinguish startup from sustained admission. Late
  slot refill can lengthen the claim span even when the claim operation
  itself is fast.
- Summed residence includes concurrently resident jobs and uses job-ms. Both
  the commit window and the post-claim drain use wall-clock ms. The direction
  counts track whether each sum and each window move together, move oppositely,
  or tie. A shared terminal XID records a batch whose jobs can finish together.
- Sparse rates, including middle 80%, include regulated arrivals. The reported
  idle gap sums the previous terminal → next submit interval. Subtracting those
  disjoint gaps from the submit window gives active submit-to-terminal time.
- Arm cells summarize the median across groups. The companion JSON retains
  every group in matrix order, per-order ratios, and the range of each paired
  throughput and duration gap. It also includes direction counts for each
  phase sum and for pairs of groups within each engine. Wins and ties use
  the exact integer windows before rounding for display.
- Elapsed claim → entry can include claim response handling, payload transport
  and worker startup. Entry → first commit can include scheduling and database
  work. First → second includes the requested hold. Second → terminal can
  include reaping, acknowledgment batching and persistence. These are
  candidate areas to investigate. Runner CPU is separately measured for the
  full runner lifecycle and has no per-stage attribution.

### Conditional Mesh bounds

For each listed segment and each non-sparse Mesh arm with monotone recorded
stamps, set that segment to zero for every job. Keep all prior timestamps,
all other per-job durations, and the window start fixed. The earliest possible
end under those assumptions is max(job terminal − job segment). Subtract it
from the recorded latest terminal to obtain the largest possible window
saving under this model. The table shows whether that optimistic bound covers
the observed full-window gap in groups where Mesh was slower. This calculation
allows jobs to overlap and relaxes shared-resource and batching constraints.
For example, removing commit → claim would allow all pending jobs to
start admission at once. Its ceiling gives the outer limit under the model
for that observed wait. A faster claim operation can recover only some of
that wait. Changes that shift other timestamps, including regulated sparse
arrivals, fall outside the model. The hold-bearing segment and producer
interval are reported as direct measurements. Improvement selection belongs
to the matrix operators.
