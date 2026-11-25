# Queue work in the direct-pool pilot

Direct binding plus four scheduler workers per runner raised runner lifetime CPU even as the runners issued fewer SQL statements. Counts come from the 12 measured Linode arms in the two opposed rotations at commit `f9002c6ce`. Each arm settled 1,024 jobs with 2,048 independent application transactions.

| Measured quantity | Block 1, service2 A / B | Block 1, direct4 | Block 2, service2 A / B | Block 2, direct4 |
|---|---:|---:|---:|---:|
| Claim calls | 166 / 168 | 115 | 168 / 167 | 110 |
| Runner SQL calls | 698 / 701 | 521 | 695 / 692 | 507 |
| Terminal transactions | 165 / 166 | 152 | 163 / 162 | 153 |
| Lifetime runner CPU, ms | 566.9 / 542.1 | 590.0 | 525.2 / 537.8 | 601.5 |
| Application backends with a whole CPU tick | 4 / 4 | 8 | 4 / 4 | 8 |

All arms recorded 136 SQL calls during the idle window. Direct4 saved at least 177 runner statements in each rotation. The CPU excess over the higher control was 23.2 and 63.8 ms. A larger number of runner SQL calls cannot explain that excess. The captures leave open the cost per call, scheduler work, and other client-side work.

We also checked the middle 512 second-write commits against [the application XID overlap bound](INFLIGHT-XIDS.md). The direct4 lower bound reached one in each block. Eight backends accumulated CPU over each run, but the XID evidence cannot establish that all eight processed assigned transactions simultaneously. The same bound reached as high as 21 in the separately measured prewarmed River middle windows.

To reproduce the table and overlap bounds, point the script at the `evidence/native` directory from the pool-supply archive in `f9002c6ce`. It verifies the released score digest and every measured archive.

```sh
python3 -B pool_supply_work.py "$native_evidence" | cmp - pool-supply-work.json
```
