# Separate Core SQLite compatibility prerequisite — UNAPPLIED

This proposal does not change R1 or R2 source, capability bits, native adapters or app code. It needs independent review before implementation. R2 scheduling qualification and the actual Core held-RPC composition remain separate.

## Problem and observed preparation effects

Unchanged Core `storage/blobs.mpl:401–403` executes `PRAGMA busy_timeout = 1000` and `PRAGMA secure_delete = ON` before beginning schema work. R1 `db/sqlite.rs:145` denies all SQLITE_PRAGMA actions. The actual scoped Core baseline returns10 with no RPC attempt. Other Core connection setup sites also use the exact1000 timeout; supporting these two operations does not establish service/Job.map compatibility.

The candidate embeds SQLite3.51.1. `prepare_probe.py` calls that exact embedded SQLite directly, outside Mesh policy. Preparing and finalizing without stepping changes busy_timeout0→1000 and secure_delete0→1. SQLITE_AUTH23 from the authorizer prevents both changes. This corroborates bundled sqlite3.c:143021 (authorizer before VFS pragma handling),143174–143192 (Btree flags changed during prepare),145189–145195 (sqlite3_busy_timeout during prepare). The existing admitted `SqlPrepare` is therefore essential, not optional. `prepare-probe.json` pins the library and results.

## Proposed bounded implementation

Recommend a **private typed configuration operation reached by exact finite SQL recognition**, preserving actual Core bytes and public exports. A new public typed initializer would be cleaner for new callers, but alone cannot fix these existing SQL calls; it requires a separate app migration and API review. Do not initialize every connection differently or silently treat the Core statements as successful no-ops.

Private types, not new effect permits:

```rust
enum CoreConnectionSetting { BusyTimeout1000, SecureDeleteOn }
enum PreparePolicy { Ordinary, CoreSetting(CoreConnectionSetting) }
```

Only the complete two existing spellings, after bounded outer ASCII whitespace trimming, are recognized initially. No comments, statement tails, prefixes, schema qualifiers, quoted names/values, alternate numeric forms, parameters, queries, other timeouts, OFF or FAST. A rejected spelling follows the ordinary deny-by-default path; it does not receive a broadly permitted authorizer. Exact matching identifies a concrete operation; its implementation must actually configure SQLite/the bounded wait policy.

Under the retained connection mutex, check exact original ownership and autocommit, validate empty parameters and execute-mode before any preparation. Nonempty parameters/tails must refuse before a setting changes; current legacy preparation-before-binding must not accidentally apply to this special path. Use canonical runtime-owned SQL for the selected enum. The authorizer's temporary per-connection preparation policy permits only the corresponding SQLITE_PRAGMA name/value/unqualified database for that canonical statement. All other authorizer rules stay unchanged. Install that policy only *inside* admitted `SqlPrepare::execute`, restore it on every return, and never leave an allow flag installed for later caller SQL. The enum selects operands; original `authority::perform` remains the only admission boundary.

Preserve separate ready-step admission. Retirement before preparation causes no configuration change. Retirement after admitted preparation may leave its finite connection setting applied, but the next step/effect must refuse under the original authority; close/rollback then owns disposal. No callback transaction is admitted wholesale. A connection configuration result grants neither future SQL nor native publication permission.

### Busy wait contract requiring its own proof

The setting is an upper budget of1000ms for an otherwise admitted connection operation. SQLite's built-in timeout counts sleeps for a locking event; it is not an aggregate operation wall-clock bound. **Do not simply enable that handler and claim a bounded scheduler wait.** Add a per-admitted-prepare/step monotonic budget and a runtime-owned busy callback. Callback inputs are the immutable original context and that operation's fixed deadline, never caller code. Every retry checks the original context; retirement/cancellation returns0 before another wait/retry. A retry stays within the same admitted exact operation and its local-settlement owner; it cannot obtain fresh authority. Sleep slices are bounded and deadline is checked again before retry. Tests must establish that this treatment of one finite accepted operation is consistent with R1's ordering.

No coroutine yield, scheduler parking or lock release occurs in SQLite's callback. If an explicit transaction or another resource scope is held, return0 immediately instead of waiting; existing error handling rolls back the scoped transaction. This is conservative SQLite_BUSY behavior, not an instruction to retry the entire callback. The callback cannot call SQLite, destroy a connection, run Host code, allocate deferred work or unwind across C. Native scheduling delays/filesystem stalls are not claimed to have a hard wall-clock guarantee.

One concrete implementation is to execute the canonical timeout configuration inside admitted prepare, record the fixed1000 budget in the connection, then install the runtime busy handler there; per-effect RAII supplies and clears the deadline. Replacing SQLite's built-in handler changes its internal busy_timeout introspection value. This unit continues to deny PRAGMA queries: it supports the Core execute operation's bounded waiting behavior, not arbitrary native introspection compatibility. If preserving that additional SQLite query behavior is required, this design needs revision rather than an invented readback value. Standalone connections retain their existing behavior.

### Secure-delete contract and durability boundary

Execute actual canonical secure_delete=ON under the admitted preparation policy, then independently admitted step; do not simulate success. SQLite's Btree secure-delete mode zeros deleted cells and also writes freed leaf pages, unlike FAST (sqlite3.c:75595–75622). That setting is connection/Btree state, not a persistent database setting. The direct reopen probe restores the build default, and Core closes its schema connection at blobs.mpl:426; other connections do not inherit this call's setting. The proposed compatibility unit must not claim all later Core deletes are configured securely. A typed initializer on every intended storage connection would be a separate reviewed migration if that is the desired product contract.

Actual overwrite durability follows the admitted deleting transaction's normal SQLite journal and commit/fsync behavior. The flag does not scrub old WAL/journal snapshots, backups, filesystem copies or flash remanence, and does not retroactively erase rows. No journal_mode, synchronous, checkpoint, VACUUM, attachment, extension or global-setting permission is added. Cancellation before commit rolls back; an already admitted commit may finish, with local settlement retained until it finishes. The next operation re-admits original authority.

## Required red/green acceptance cases

1. Real generated scoped Core account/schema and load paths: retain current refusal red; candidate reaches the intended RPC using unchanged Core SQL and source. Only then rerun the actual held-RPC/new-load composition; shared-service refusal may be the next independent barrier.
2. Prepare-only boundaries: injected retirement before prepare leaves both settings unchanged; retire after admitted prepare and before step refuses the step. Authorizer restored after success/error/panic/close. Wrong owner and unsupported capability cases refuse before touching SQLite.
3. Exact grammar/parameter matrix: two positives; all alternate settings, schema qualifiers, tails/second statements, comments, embedded NULs, quoted/malformed values, bound parameters and query entry points remain denied with no partial first-setting effect. Exercise every execute variant sharing this boundary.
4. Real two-connection contention: fresh admitted operation succeeds when lock releases within budget; expiry returns BUSY without an open transaction or leaked handle. Retirement during a slice starts no subsequent retry/write. No wait under an explicit transaction/resource scope; rollback is observable and another invocation progresses. Record actual elapsed intervals with scheduling tolerance; do not turn a timing sample into a hard bound.
5. Real file deletion: use disposable identifiable data on the configured connection, include overflow/freelist pages, commit/close/reopen and inspect main-database bytes. Include rollback/crash and WAL/journal retention controls; only claim what was checked. Reopened/new connection configuration must be independently verified, not inherited from prior calls.
6. Preserve all31 R1 cases (especially hot-journal open/prepare admission and distinct step/commit), mask1, original176 rows/audiences/limits/bytes and legacy callbacks. Independent source/API/effect review and exact tool/source/artifact pin precede any Core adoption.

This proposal deliberately leaves the accepted PRAGMA denial in place today. No successful direct SQLite research call is a Mesh permission or product compatibility receipt.
