# Where the first handler entry falls within a SLOW claim

Across five canonical Mesh blocks, 89–92% of accumulated time between the database claim stamp and handler entry elapsed before the **first handler in the same stamped group** entered. A change limited to launching the remaining handlers closer together would address only 13–19 normalized slot-ms per block on the captured timelines. Earlier processing of the whole claim has more room. Four later same-shape inline controls put the two terms at 129–147 and 18–20 normalized slot-ms.

| Linode block | Mesh stamp to first entry | Mesh later entry within group | River stamp to first entry | River later entry within group |
|---|---:|---:|---:|---:|
| 1 | 134.0 | 16.5 | 31.8 | 31.7 |
| 2 | 158.1 | 13.4 | 42.6 | 43.6 |
| 3 | 141.8 | 17.6 | 70.1 | 22.3 |
| 4 | 138.8 | 15.9 | 47.1 | 54.2 |
| 5 | 149.9 | 19.0 | 40.4 | 26.1 |

All values are accumulated job milliseconds divided by 64 slots. For each distinct database attempt stamp, take the earliest entry among jobs with that stamp. Split each job's interval at that entry, then sum the two parts independently. River's attempt timestamps come from its own claim implementation. Its observed groups reached 64 jobs. Mesh's SQL assigns a common integer millisecond after sizing the picked contexts and uses it in the grant UPDATE. Its groups reached 16. Millisecond truncation can add up to eight normalized ms to the Mesh first-entry term over 512 jobs. The within-group term uses only handler timestamps. The differences describe the two implementations at their recorded boundaries. They do not establish a directly recoverable wall-time difference. [Ledger timestamps](../../native/ledger.py) · [Stock grant SQL](../2026-11-25-dense-logical-grant-key-native/stock.sql)

In every canonical Mesh block the median full 16-job group's first entry was 15.4–18.1 ms after its stamp. Medians for singleton groups were 2.5–7.2 ms. The first interval includes updating the picked rows, sending and validating the reply, confirming COMMIT, scheduling and reaching the first handler. Grant width correlates with workload phase and available slots. These clocks cannot isolate a byte-transfer cost or an index-scan cost.

The whole-arm PostgreSQL counters reported 603–639 ms inside the checked claim SELECTs and 254–271 ms in the 512 scalar settlement SELECTs per canonical block. They include idle calls and work that can overlap the handlers. The ledger and counters agree on the number of stamped positive grants after accounting for extra marker rows. A separate SLOW plan test can now check whether the UPDATE visits broadly at width 16 before pricing the bounded lookup. The booked width-32 test measures a different change.

Run `python3 -B claim_entry.py | cmp - claim-entry.json` in this directory. The replay checks the original raw SHA-256 for all 14 arms, the 100 ms committed holds and both independent writes per job. [Raw-derived measurements](claim-entry.json) · [Full slot partition](HOLD-BUDGET.md)
