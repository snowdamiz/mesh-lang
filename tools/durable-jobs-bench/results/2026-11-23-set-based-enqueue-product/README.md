# Integrated set-based Jobs enqueue: local gates and native correctness request

Commit `84e1cec9` on local branch `t3/set-based-enqueue-product` builds on
`030b45e6`, the current Jobs product with the optional set-based claim
grant. It adds the opt-in writer `MESH_JOBS_SET_BASED_ENQUEUE=1`. The default
remains the ordered scalar writer. It ports the natively measured
`ce5e551d` writer unchanged, plus the qualified
[EXTERNAL layout hunk](../2026-11-23-slow-external-compat/frozen/ce5e551d-external-layout.patch)
(SHA-256 `5fcfa7ae…ab75a4`). [Proof notes](../../../../proof/jobs-set-based-enqueue/README.md)

## Local gates on the committed source (macOS ARM64)

Every gate ran on a source identical to the commit
([diff](evidence/local/gated-source.diff.gz), [hashes](evidence/local/gated-source.sha256)).

| Gate | Exit |
|---|---:|
| PostgreSQL 17.11 compatibility, roles, claim and enqueue relay proofs | 0 |
| PostgreSQL 18.7 compatibility and roles (the enqueue relay proof reports that it skipped) | 0 |
| PostgreSQL 19.0 compatibility and roles (the enqueue relay proof reports that it skipped) | 0 |
| rustfmt, workspace build, workspace tests (4,821 passed, 0 failed), diagnostics-minimal | 0 |
| effect-transaction on 17.11, and effect-fence on 17.11, 18.7, and 19.0 | 0 |

On 17.11 the relay proof passed seven cases. They are partial row, missing
row, wrong ordinal, non-insert receipt, COMMIT dropped, COMMIT reply lost,
and ReadyForQuery withheld. Each case also checks the database state and the
request-key replay.

The first 17.11 attempt is retained as
[`pg-17.11-attempt1-forged-commit-tag`](evidence/local/pg-17.11-attempt1-forged-commit-tag.log.gz).
It also had a forged-tag case: the relay rewrote a successful COMMIT's tag
to `ROLLBACK`. PostgreSQL had committed all 20 jobs and receipts, and the
client returned them. The shared Jobs commit helper infers a rolled-back
COMMIT from the aborted transaction state, as PostgreSQL reports it, so it
does not check the tag. The case was removed from the enqueue proof. That
helper behavior is shared with the ordered writer and is unchanged. The
18.7 and 19.0 binaries came from `target/integrated-uncut-delivery/postgres`.

## Native evidence for the pre-integration writer

The controller owns these packets. This directory references them and does
not modify them.

| Packet | Status |
|---|---|
| `../2026-11-23-set-based-enqueue/evidence/native-setup-3b09dc3d0e23`, setup qualification | committed in `03dfb958` |
| `../2026-11-23-set-based-enqueue/evidence/bulk-enqueue-v1-slow`, V1 slow, 5 groups | committed in `bad1a384` |
| `../2026-11-23-bulk-enqueue-v2-short`, inventory, orders and replay | committed in `119493e5` |
| `../2026-11-23-bulk-enqueue-v2-short/evidence/bulk-enqueue-v2-short`, raw V2 short: failed dispatch `29083af379b0` (group 2 CPU steal 1.06% over the 1% limit) and unchanged retry `8eef0f9b8239` | **not yet committed by its owner**; [read-only manifest of 187 files](evidence/local/v2-short-raw.sha256) |
| Sealed source packets | committed in `75bbb906` |

With the same ELF, bulk submitted faster than scalar in all ten pairs. V2
short had a median of 1.212× and V1 slow 1.045×. Commit rates were 1.033×
(4/5) and 0.970× (1/5). These figures describe ELFs built from `ce5e551d`,
not from this commit.

## Native correctness request

[`replay/correctness.py`](replay/correctness.py) runs on the controller
after an exclusive SETUP grant named `set-based-enqueue-product`. It streams
[the source archive](native/set-based-enqueue-product-84e1cec9.tar.gz)
(SHA-256 `8fce2d3d0e90d51dd19a332914ac5728be0b8af7d9da7f546db98e8663e082f7`)
to [`replay/native-correctness.sh`](replay/native-correctness.sh). That
script holds both host locks for the whole run and performs these steps:

1. Fetch crates offline.
2. Build the release `libmesh_rt.a` and `meshc` and record their hashes. This
   is a new build identity, with no timing.
3. Prebuild every test binary as root.
4. Run as `nobody`:
   - `verify-postgres-jobs.sh --compatibility 17.11 --parity-driver-check`,
     which runs all Jobs tests and both relay proofs and must report seven
     enqueue relay cases
   - `--effect-compiled`, Linux-only, as a no-regression check. This fixture
     enqueues through the scalar function.

The archive passed an offline `cargo check --workspace --tests` and the
parity self-check after extraction. The role proof needs git history, so it
stays local, where it passed on all three versions. This request makes no
timing or release claim. No ELF built from this commit has run on the
Linode host yet.
