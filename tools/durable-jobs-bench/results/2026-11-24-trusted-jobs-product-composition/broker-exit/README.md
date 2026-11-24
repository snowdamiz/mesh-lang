# Reading the producer’s final reply

The original dense group 3 warmup raised `RuntimeError` while reading the producer’s stop reply. Its output file ended in a complete `{}\n` response and the error log was empty. All four engines reran in the original order under the original inventory. The successful retry supplies group 3’s measured scores. [Failed Linode arm](../evidence/native/timing-closure-retry/dense-stock/blocks/03-noop.1795496194789458000/warmup-product/warmup-mesh/raw.tar.gz) · [Completed dense series](../evidence/native/timing-closure-retry/dense-stock/summary.json)

The original reader could encounter EOF, then observe that the producer had exited. A child that wrote its final response between those two operations left a complete reply in the file. The corrected reader retries the same offset once after observing exit. A missing or partial final line still fails. The caller also checks the producer’s exit status. [Reader correction](final-reply.patch)

Run `python3 -B replay.py` in this directory. The untimed regression verifies the original raw archive and source digests. A real child writes a complete line or finishes a partial line during the process-exit check. Both schedules reproduce the failure on the pinned reader and pass on the corrected source. The archived incident is consistent with these schedules. [Regression](replay.py)

The corrected checkout passed all 16 benchmark-tooling tests and the parity-driver and PostgreSQL 17.11 compatibility checks. The Linode repeat used its original pinned reader.
