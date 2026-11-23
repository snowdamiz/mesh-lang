# Local Linux receipts

All three guests used PostgreSQL 17.11 on ARM64 Linux with verified TLS. The fixture and recorder ran as UID 65534. The input manifest beside each log records that run's code revision by hash.

- [Two-core Linux smoke](linux-03/serial.log) reproduced both legacy guards and accepted 60 of 60 frames per engine for the earlier candidate. The fixture retired a backend between the start and end SQL snapshots and verified that the candidate excluded it.
- [Two-core deadline rejection](linux-05/serial.log) reproduced both Mesh startup counts and the Oban fixed-roster guard. It accepted all 60 Mesh frames before rejecting an Oban frame that exceeded the existing 25 ms deadline.
- [Spare-core deadline rejection](linux-08/serial.log) put the workload and database processes on CPUs 0 and 1 and the recorder on CPU 2. It rejected a Mesh frame that exceeded 25 ms during the timed loop. The candidate keeps that admission limit.

A later local retry waited 60 seconds for the shared Linux proof lock and exited before boot. The lock was held by the separate partitioned-admission timing series. These receipts are identity and failure evidence. Native timing qualification remains open.
