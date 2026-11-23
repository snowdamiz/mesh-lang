# Set-based fresh-process batch claims

The option is `claim_batch: true` with `claim_batch_strategy: "set_based"` on a fresh-process runner. The runner context validates the option before starting. The Jobs schema version stays at 1 with an additive claim function. The schema owner migrates first and can apply the [scoped opt-in grant](../../scripts/fixtures/jobs-set-based-claim-grant.sql) to an existing worker. The baseline role installer also works with older Jobs schemas. Reapplying migration preserves explicit grants, and the new function revokes PUBLIC execution even when the database uses the PostgreSQL default ACL.

The new function takes a queue row's `FOR NO KEY UPDATE` lock and measures headroom with a fresh READ COMMITTED snapshot. Single-claim and cursor runners use the same queue lock. That lock allows the producer's foreign-key key-share check to proceed. The set-based claimant never waits for a namespace advisory lock while holding the queue row. Namespace pause serializes at the advisory lock, and legacy direct state transitions still pass through the namespace fence trigger. Queue pause and scale use the same queue row as admission. Job rows use `SKIP LOCKED`; cancellation, renewal, wakeups, expiry and recovery continue through their existing paths.

The set-based claim keeps the ordered cursor fallback for contexts outside its conservative sizing envelope. On both routes, the client checks the result count, shape, byte lengths, ordinals, identities and grant context in the open transaction. It then requires an exact COMMIT tag and idle ReadyForQuery. Malformed or truncated pre-commit replies spend no attempts. The before-COMMIT cut leaves attempt zero. The after-COMMIT cut may consume a claim, and the runner suppresses every launch from that uncertain reply. Normal lease recovery applies from that point. Settlement remains individual per reaped attempt. The selected ordinary shared-process runner keeps its existing cursor strategy, connections and renewal logic.

## Reproduce

Run the usual workspace and disposable PostgreSQL proof commands from the repository root. The compatibility gate adds a real TLS relay for both the set-based route and its checked cursor fallback. It checks a partial row, missing row, malformed ordinal, malformed context, wrong row count, invalid ReadyForQuery, a batch over 32 MiB, both COMMIT loss cuts, and a wrong COMMIT tag. The compiled durable Jobs test exercises the opt-in with multiple fresh-process runners, a persisted pause and scale change, and distinct terminal XIDs.

```bash
cargo test --workspace --locked
PG_BIN_DIR=/path/to/postgres/bin bash scripts/verify-postgres-jobs.sh \
  --compatibility 17.11 --parity-driver-check --j6-roles
```

Repeat the database command on PostgreSQL 18.7 and 19.0. The Linux x64 or ARM64 protected fixture also accepts `MESH_EFFECT_SET_BASED_BATCH=1 bash scripts/verify-postgres-jobs.sh --effect-compiled`.

The five no-op and five short matched Linux rotations under `tools/durable-jobs-bench/results/2026-11-23-admission-v3-strict` describe the exact experimental ELF from `bd7851b9398198955b4c8df630310a29b38cddd4` (SHA-256 `cf4f22be4995fd5f7c00a45e1ea97753c1c65577cc34142195f6d98d11087483`). Paired median commit-to-terminal rate improved by 10.6% on no-op and 6.3% on short. Paired median submit-to-terminal rate improved by 8.8% and 6.7%. All ten pickup p99s improved. The opt-in product source and its next native ELF have independent build and proof identities.
