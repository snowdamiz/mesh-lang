# Native empty recovery pilot

This pilot tests the exact stock and candidate recovery functions from public `b3557273d72b` on the archived hot grouped 2×32 Mesh lane. It pins the compiler and ELF through the existing public artifact manifest, canonical 1,024-job input, PostgreSQL 17.11 binary, two CPU IDs, Linode boot ID, kernel and disk. The controller owns staging and the existing authenticated native lease. [Controller packet](controller-packet.json)

The stock shakedown first runs the PostgreSQL probe in a disposable clone of the migrated database. It uses committed, distinct 1,024-byte JSON arguments at depths 0, 1,024 and 16,384. The native plans execute the exact extracted UPDATE under forced custom and generic modes with buffers and WAL. The separate nested `pg_stat_statements` rows come from the actual public recovery function under the worker role. The probe tests empty, a full 16-row page, mixed eligible and excluded rows, and an empty result while the other worker holds the due page. It also verifies two independently committed recoveries, expiry settlement and zero-row transition-trigger firing. The statement trigger lives inside a rolled-back proof transaction. [Probe](probe.py)

Both matched arms start on fresh databases with the original indexes, triggers, role grants, queue settings and durability. The only treatment is the recovery predicate. Both arms enable nested stats for the queue role and reuse the existing idle and terminal observer round trips. The layout guard must pass before and after, and the archived ledger plus statement counters must show eight true batch calls, 1,024 returned rows, positive writer WAL and 1,024 committed terminal jobs. The grouped ledger verifier checks both runner processes, the application and settlement transactions and the TLS sessions.

The initial matched pair excludes the shakedown clock. The follow-up runs two opposite-order pairs only when the initial pilot reaches the packet threshold without an empty or due-path regression signal. A material retained-history signal can justify separately labeled dense arrivals on the same-namespace completed corpus. That follow-up requires an observer that excludes the fixture rows from workload terminal counts and otherwise preserves the benchmark clocks. Any peer score must use equal retained history on every engine. This packet leaves existing baseline bookings and scoring untouched.

Nested timings are summed submit-to-observer statement elapsed and can overlap. The stable backend CPU bracket covers all operations on those backends. Any small canonical saving will be reported in its measured scope. A single pilot pair cannot establish end-to-end improvement or a peer-gap reduction.

```sh
# Untimed local correctness and offline archive replay
python3 -B local_check.py /opt/homebrew/opt/postgresql@17/bin
python3 -B controller.py check
# Sole controller, under the matching Linode grant
python3 -B controller.py timing
python3 -B controller.py replay
python3 -B controller.py advance  # only if the pilot qualifies
```
