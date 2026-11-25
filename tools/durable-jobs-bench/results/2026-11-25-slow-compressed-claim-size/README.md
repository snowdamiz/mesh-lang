# Compressed SLOW payloads on the public checked grant path

On PostgreSQL 17.11, all 512 original SLOW arguments had a raw JSONB size of 16,472 bytes. PGLZ left them uncompressed. LZ4 compressed every argument, reducing their total reported column size from 8,431,616 to 8,171,728 bytes. The public set-based function from `b3557273` returned a fallback marker on **each of 32 full LZ4 batches**. Its runner confirms that transaction, runs the due probe when enabled and calls the checked cursor in another transaction. With PGLZ the set-based function granted the jobs directly. The earlier four-engine storage transfer uses a V1 binary with the cursor implementation. Its storage comparison has a different claim path.

The SQL witness keeps the released size check for uncompressed arguments, metadata and tags. For compressed values it reads PostgreSQL's raw datum size before rendering and requires the three raw sizes to sum to at most 65,536 bytes. The guarded witness calls the public set-based function when the enqueue layout has drifted or the caller lacks EXECUTE on the effect-key helper. In the canonical layout it granted all 512 jobs under each policy with the same ordered IDs, context bytes, and reported lengths as the appropriate stock route. A 1,048,599-byte argument stored in 4,146 bytes still forced fallback.

Two controls matter when evaluating a public opt-in.

- With a custom effect-key function, the cursor returned the custom key. Changing the size check alone produced the stock formula's key. The layout check diverted this case to fallback.
- Revoking EXECUTE on the helper preserved the layout digest. The separate privilege check diverted the restricted caller to fallback, and the checked cursor raised the expected insufficient-privilege error for the helper.

This is an untimed SQL witness on a disposable cluster using a private PG17 C helper. The harness supplies the original job arguments directly, assigns repeatable IDs and retires each sixteen-job page by SQL inside a rolled-back transaction. Full production migrations, restricted caller grants, concurrent DDL, runner fault handling and the elapsed cost of the layout check still need qualification for any product candidate. The witness measures neither claim latency nor peer throughput. The original V1 PGLZ/LZ4 qualification can proceed independently.

Run from this directory with an LZ4-capable PostgreSQL 17.11 installation, its server development headers and a C compiler.

```sh
python3 -B proof.py | cmp - proof.json
```

[Deterministic result](proof.json) · [SQL proof](proof.py) · [Source of the size accessor](raw_size.c)
