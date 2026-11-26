# Public dense Jobs with individual settlement

Six native rotations ran 1,024 jobs per arm on CPUs 0 and 1 with two 32-slot runners and PostgreSQL 17.11. Each Mesh job had two independent application XIDs and its own terminal XID. The three Mesh lanes ran the same Jobs proof ELF. The hot lane enabled the application pool URL option.

Times are median [min, max] in milliseconds. The middle intervals span completion ranks 103–922 separately for second application commits and terminal commits.

| Lane | Submit to terminal | Enqueue commit to terminal | Middle second writes | Middle terminals |
|---|---:|---:|---:|---:|
| hot | 1353.6 [1335.3, 1443.8] | 1194.9 [1172.4, 1258.2] | 914.9 [890.7, 941.2] | 917.1 [909.0, 941.9] |
| checked-a | 1488.7 [1472.5, 1566.4] | 1325.7 [1310.4, 1370.2] | 1022.2 [1004.1, 1073.2] | 1019.5 [1002.7, 1062.8] |
| checked-b | 1497.0 [1461.4, 1573.2] | 1325.4 [1304.8, 1372.3] | 1021.1 [971.3, 1038.2] | 1026.3 [968.2, 1040.6] |
| river-2x32 | 1017.7 [1009.7, 1024.8] | 945.1 [921.4, 948.9] | 345.7 [312.5, 357.4] | 295.3 [223.6, 426.4] |
| oban-2x32 | 1337.1 [1280.9, 1537.3] | 1185.4 [1147.6, 1365.5] | 835.8 [800.8, 923.6] | 826.0 [805.6, 929.0] |
| apalis-2x32 | 729.2 [684.2, 828.7] | 692.4 [647.8, 783.1] | 498.3 [481.1, 569.2] | 499.1 [466.7, 563.6] |

Paired millisecond differences are hot minus the named lane. Negative differences favor hot. Parentheses show the number of rotations hot won.

| Comparison lane | Submit to terminal | Enqueue commit to terminal | Middle second writes | Middle terminals |
|---|---:|---:|---:|---:|
| checked-a | -135.4 [-222.3, -41.9] (6/6) | -137.2 [-181.5, -55.5] (6/6) | -104.9 [-163.5, -91.8] (6/6) | -94.3 [-150.8, -83.2] (6/6) |
| checked-b | -136.8 [-170.5, -67.6] (6/6) | -135.5 [-163.9, -105.9] (6/6) | -102.4 [-118.2, -80.6] (6/6) | -104.6 [-113.8, -48.7] (6/6) |
| river-2x32 | 337.9 [324.6, 418.9] (0/6) | 254.2 [228.6, 309.3] (0/6) | 571.7 [566.7, 583.8] (0/6) | 615.2 [515.5, 696.0] (0/6) |
| oban-2x32 | 1.2 [-174.3, 162.8] (3/6) | -11.4 [-164.4, 110.6] (4/6) | 73.9 [-13.8, 140.4] (1/6) | 91.1 [-17.0, 136.2] (1/6) |
| apalis-2x32 | 639.2 [534.3, 699.4] (0/6) | 516.6 [418.0, 543.5] (0/6) | 416.3 [343.8, 452.7] (0/6) | 434.2 [345.5, 453.3] (0/6) |

- hot recorded 4–4 checkout probes per arm and 41–41 peak PostgreSQL connections.
- checked-a recorded 2048–2048 checkout probes per arm and 41–41 peak PostgreSQL connections.
- checked-b recorded 2048–2048 checkout probes per arm and 41–41 peak PostgreSQL connections.

Hot passed the preregistered four-of-six duplicate-control gate with 4/6 qualifying rotations.
