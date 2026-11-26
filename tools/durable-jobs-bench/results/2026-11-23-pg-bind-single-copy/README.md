# PostgreSQL Bind: encode once into the outbound message

Base `29abb9cb` (its `compiler/mesh-rt` PostgreSQL sources match `8ff6eb29`).
The candidate is [`pg-bind-single-copy.patch`](pg-bind-single-copy.patch), a
clean single commit. `git apply --check` passes on `8ff6eb29`. It has not been
merged into the product and still needs review.

## Change

`write_bind_values` used to check sizes, build the Bind body in a new `Vec`,
and copy that body into the request. The candidate:

- appends the frame directly after a single `reserve`. The reservation covers
  the frame plus the suffix the caller always appends next: Execute+Sync is
  15 bytes, and Describe+Execute+Sync is 22 bytes.
- adds up the frame size in `u64`, so a 32-bit `usize` cannot wrap before the
  64 MiB frame check. A new unit test covers 257 maximum-size values.
- still runs every validation step before writing any byte. A rejected Bind
  leaves the caller's buffer bytes and capacity unchanged.
- updates all callers: `execute`, `query_values`, the text query path, and the
  fenced `EffectSession::query`. The effect session is only compiled on
  `linux-gnu` 64-bit targets and in unix tests.

Round trips and message order do not change. Only the place where the exact
bytes are built moves.

## Correctness evidence

- `evidence/differential.txt`: the stock and candidate functions are
  extracted from source and compiled side by side. They give byte-identical
  output for the empty, mixed text/binary/NULL, 32767-NULL and exact 64 MiB
  frames. Both give the same errors and keep the prefix and capacity for:
  - a frame over the limit,
  - a value over the limit,
  - 32768 parameters,
  - a 32-bit sum overflow.

  A fixed typed vector also matches. `write_wire_all` partial-write and
  failed-flush behaviour is unchanged.
- `evidence/wire.json`: a loopback bridge that verifies TLS 1.3 on both legs
  re-frames every message and splits headers. For both workloads below, the
  compiled Mesh stock and candidate binaries produced the **same full
  frontend transcript digest**:
  - the large mixed workload: 54,265,825 frontend bytes;
  - the original large Jobs batch (input sha `9d7f130f…`, 64 jobs): 33,608,149
    frontend bytes, 130 `PBDES` exchanges.

  Results, row counts, commit timestamps and `pg_stat_statements` call counts
  were checked against PostgreSQL 17.11 with `fsync=on` and
  `synchronous_commit=on`.
- `evidence/verify-postgres-jobs-17.11.txt`:
  `scripts/verify-postgres-jobs.sh --compatibility 17.11` passed on the
  candidate snapshot. That covers the PG compatibility tests, PG ownership,
  db values, migrate, `db::pg` tests, `jobs::` with ignored tests included,
  `e2e_durable_jobs`, and the claim-admission and set-based-enqueue proofs.
- `evidence/mesh-rt-tests.txt`: `cargo test -p mesh-rt --lib --test library
  --test library_memory` passed: 1600 passed, 53 ignored. `cargo fmt --check`
  and the whitespace guard were clean.

## Performance

Arms: `stock` (full workspace release build), `stock-b` (clean rebuild of
`mesh-rt` alone), `optimized-a` and `optimized-b` (two clean rebuilds, with
identical archive digests). Eight interleaved groups were run with balanced
orders, under the repository timing locks. All numbers come from
`summary.json`, which `summarize.py evidence` regenerates.

**Release encode, measured in isolation** (`micro.rs`, process CPU per
encode of the full Parse/Bind/[Describe]/Execute/Sync request). The
candidate was lower in 8 of 8 groups for every shape. Stock-vs-stock control
medians were within ±6%.

| shape | bytes | encode CPU | peak live alloc | allocs + reallocs |
|---|---|---|---|---|
| short `INSERT` (2 text) | 123 B frame | −23% | 247 → 128 B | 8 → 6 |
| scalar 8 MiB text | 8.0 MiB | −68% | 16.0 → 8.0 MiB | 9 → 6 |
| mixed 6 MiB binary + 2 MiB text + NULL, typed results | 8.0 MiB | −62% | 16.0 → 8.0 MiB, no realloc | 4 → 1 |
| Jobs `mesh_jobs.enqueue` call (real captured values, 524,611 B payload) | 512.5 KiB | −54% (≈16.7 → 7.7 µs) | 1.0 → 0.5 MiB | 9 → 6 |

The copied body bytes go to zero. For the stock frame, the copy equals the
frame size minus 5.

**Full compiled Mesh against local PG17.11 with verified TLS** (per-group
paired medians, optimized arms vs stock arms):

| workload | peak RSS | process CPU | wall |
|---|---|---|---|
| large: 12 transactions × (7.9 MiB scalar insert, 9.0 MiB mixed insert, 9.0 MiB typed query) | **−10.2%, 8/8** (88 → 79 MiB; control 0%) | −7.0%, 6/8 (control spread −15…+26%) | −12.9%, 5/8 (control spread −19…+52%) |
| short: 384 transactions × 3 calls | 0% | within noise (control spread −52…+89%) | within noise |
| original large Jobs batch, true producer session (64 enqueues, 1 commit) | **−1.4%, 8/8** | −2.8%, 6/8 (control −16…+10%) | within noise |

The host's 1-minute load average was 36–71 during the live groups because
other lanes were running. The repository timing locks were held, but builds
outside those locks still competed for CPU. Live CPU and wall effects are
therefore **not claimed**: the peak-RSS reduction is the only live result
that clears its controls.

## Jobs pricing

The original large batch sends 64 `mesh_jobs.enqueue` Binds totalling
33,583,616 frame bytes. Together with the 64 admission Binds, stock copies
33.6 MB of Bind bodies per batch. The candidate copies none and saves about
9 µs of encode CPU per enqueue, roughly 0.6 ms per 64-job batch. The batch's
wall time is about 3 s, so that is around 0.02%. The saving is real but far
too small to move queue throughput, and no throughput claim is made. The
measurable gain for Jobs is the lower transient peak memory per large
producer request.

## Verdict

Worth integrating as a general driver improvement:

- identical bytes on the wire;
- the 32-bit size check is now explicit;
- encode CPU drops 2–3× for large values;
- transient peak memory per large Bind halves.

It is not a queue-throughput change. It needs review before any product merge.

## Replay

```
python3 -B replay.py --root <checkout> --candidate <patched source> \
  --scratch <scratch> --arms <dir with {stock,stock-b,optimized-a,optimized-b}/libmesh_rt.a and stock/meshc> \
  --out <new dir> --score-jobs
python3 -B summarize.py <new dir>
```

This needs PostgreSQL 17.11 binaries on `PATH` and `rustc` (2024 edition).
`wire.py` turns off only Python 3.13+'s `VERIFY_X509_STRICT` on its upstream
leg, because the harness CA has no Authority Key Identifier. The CA chain and
hostname are still verified.
