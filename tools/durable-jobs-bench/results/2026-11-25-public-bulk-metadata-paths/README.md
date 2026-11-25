# SLOW bulk enqueue metadata reads

The candidate changes the four metadata queries before insertion in PostgreSQL 17's `mesh_jobs.enqueue_batch_v1`. The public inline broker calls this function eight times for 504 jobs. Each call supplies 63 job specifications, each carrying a 16 KiB payload. Eight scalar enqueues complete the original 512-job batch.

The released function expands each batch into whole job records to check uniqueness and replacement policy, find duplicate request keys, look up existing receipts, and register queues in input order. The candidate walks array positions for those queries and projects only the required JSONB fields. The receipt lookup materializes its small request keys once. The writer still expands complete records for the MERGE and returns ordered receipts. The patch targets the SQL installed from `b3557273d72be3802bf189b284e7caaf5f3caa0f`. [Candidate patch](candidate.patch)

## What the existing measurements price

In the single-runner inline DEFAULT controls, serial producer SQL occupied 319.8–359.8 ms. That total includes the durable inserts, scalar tail and PostgreSQL storage work. The existing measurements do not assign elapsed time to the four metadata queries. If all four expansions visit each bulk item, their full records collectively contain 31.5 MiB of logical application payload. This counts payload present across those queries. It gives neither the physical bytes copied nor milliseconds saved. Read the source with `git show 8441b97d2:tools/durable-jobs-bench/results/2026-11-25-slow-inline-producer-floor/README.md`.

## Qualification

Direct paths must agree with the released field operators on missing keys, JSON null, malformed shapes, duplicate and existing receipt keys, and queue registration order. The held migration and admission fences, role privileges, first error, rollback, replay identity and application writes are part of the comparison. Query plans should identify any repeated detoast or misestimated receipt join, particularly for a stored or generic-plan argument.

The intended matched experiment times one storage policy at a time with the retained runner and producer. The decisive clocks run from submit and from durable enqueue to the last settlement. An earlier enqueue commit alone can coincide with slower drain. Only a fresh native comparison can establish the effect of this patch. [Isolated codec results](../2026-11-25-slow-lz4-isolated-qualification/RESULTS.md)
