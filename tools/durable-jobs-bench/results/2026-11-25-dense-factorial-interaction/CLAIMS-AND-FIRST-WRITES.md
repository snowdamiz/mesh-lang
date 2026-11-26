# DENSE runners had work waiting on both sides of a claim

Across five native 8×8 rotations, public Mesh B took longer than River to get from the 256th to the 768th first application commit. Mesh had at least one entered job awaiting its first commit for all but 0–3.745 ms of each window. River went 74–126 ms with no such job and still completed the 512 writes sooner in every rotation. Supplying additional runnable jobs alone has limited appeal until we understand the rate at which the current work reaches its first durable write.

| Rotation | Middle first writes, Mesh / River / Oban (ms) | Mean jobs entered, first commit outstanding, Mesh / River / Oban | Median entry to first commit, Mesh / River / Oban (ms) |
|---|---:|---:|---:|
| 1 | 512 / 316 / 648 | 9.52 / 2.48 / 6.39 | 6.90 / 0.63 / 6.51 |
| 2 | 1,237 / 299 / 649 | 15.04 / 1.76 / 6.07 | 17.47 / 0.55 / 6.36 |
| 3 | 522 / 300 / 671 | 10.93 / 1.92 / 6.40 | 8.26 / 0.60 / 6.40 |
| 4 | 541 / 277 / 643 | 10.08 / 1.75 / 6.30 | 8.49 / 0.50 / 6.39 |
| 5 | 532 / 301 / 637 | 10.55 / 1.65 / 5.68 | 8.09 / 0.51 / 5.76 |

The mean counts integrate each job's entry-to-first-commit interval over that engine's window, including jobs that cross either end. The medians use the 512 jobs whose first commit landed inside the half-open window. The River worker IDs come from its physical attempt ledger joined by job key and XID. Every arm used eight worker PIDs and recorded 2,048 distinct application XIDs. We include the slower second Mesh rotation. [25 released archives and exact counts](peer-first-write-residence.json) · [Full submit and drain scores](../2026-11-25-public-dense-late-capacity/peer-comparison/RESULTS.md)

## The clocks cover different application paths

Mesh records entry before parsing the handler context, resolving the application pool and issuing the checked attempt INSERT. River records entry inside its typed Work method and retains one checkout for two independent writes. The Mesh attempt records job identity and generation and uses a conflict target for duplicate attempts. River binds a timestamp and omits that conflict clause. Oban also retains a checkout across its two independent writes. The paths all produce durable records. The entry intervals span application work, scheduling, pool service and database waits, with different work occurring before each entry timestamp.

The public B database recorded 55.1–84.4 ms of total execution time for all 1,024 attempt INSERTs in each rotation. That sum excludes planning and transaction completion. The 6.90–17.47 ms per-job medians above cannot be assigned to the INSERT's index check. The earlier 2×32 direct pool binding moved much of the apparent entry delay before entry and advanced the middle effects by just 1.6–9.2 ms. Measure the change in durable ranks whenever changing where a handler blocks. [Mesh handler](../2026-11-24-e4156740-trusted-jobs-native/overlay/fixture/main.mpl) · [River handler](../../go/worker.go) · [Direct-pool comparison](../../../../../t3-bdc4c647/tools/durable-jobs-bench/results/2026-11-25-public-dense-fixed2-app-pool/README.md)

## Overlap with the queue prelock

The separate claim observer records partial requests from send until the checked queue-lock reply. Throughout both measured middle **effect** windows, at least two runners had requests outstanding and a different runner had an entered handler still awaiting its first commit. At least four waited simultaneously for 374–427 of 470 ms forward and 388–450 of 500 ms reverse. The mean numbers of runner PIDs with entered jobs awaiting a first write were 3.07 and 3.28. A pending request includes transport and server response time. Entered handlers may be descheduled or waiting on the database.

The sweep shrinks or expands each interval separately for the monotonic-to-UTC offset bounds and timestamp precision. It counts distinct PIDs, clips to effects 256–768 and admits empty or unsuccessful prelock replies. The observer failed its perturbation controls. This describes concurrency on the instrumented arms. Quantifying a throughput improvement requires new matched end-to-end controls. [Overlap bounds](claim-work-overlap.json) · [Claim phase definitions and archive pins](CLAIM-PHASE-BUDGET.md)
