# Isolated PostgreSQL 17.11 for the ordinary slow codec crossover

This follows the original [ordinary-slow-storage-transfer hold](../2026-11-23-slow-storage-transfer/request.json). It uses the frozen 512 × 16 KiB payloads, 100 ms holds, one runner with 64 slots, eight Mesh application lanes, a 68-connection cap, and the original four native engine artifacts. Mesh uses the compiled ordinary V1 bulk enqueue path and the `claim_batch_v1` cursor. The public `acf1` and `b355` claim paths have separate evidence. The cap68 and cap71 refill studies and the existing DEFAULT/EXTERNAL comparison remain separate.

The historical six-block Darwin comparison saved a paired median 105 ms of Mesh producer PostgreSQL CPU and 159 ms to enqueue commit. LZ4 beat both PGLZ duplicates on the full submit-to-settlement clock in two blocks. In the four losses, the interval from the 64th to final first application commit accounted for 107, 88, 274, and 295 ms of slowing relative to the fastest full-submit PGLZ duplicate. These landmarks describe the drains. Peer rankings require fresh native measurements. [Original workload and results](../2026-11-23-slow-storage-transfer/README.md)

## Source and native custody

The [historical bootstrap](../../native/bootstrap) from commit `6f5e3d410` has SHA-256 `f209b1e6e263c41565db172baa4149efe1e00ce282d312c5ef22325bb92a694e`. The retained Ubuntu 24.04 AMD64 build log has SHA-256 `3bc84ad515a0359fe035cefecf3724d62065cd72165c7921411b023d63c82946`. It records GCC, Bison 3.8.2, Flex 2.6.4, pkg-config 1.8.1, OpenSSL 3.0.13, ICU 74.2, zlib 1.3 and Readline 8.2. The bootstrap uses the PostgreSQL 17.11 release archive with SHA-256 `dd27f2b3c59e73ed14aa3324901242bf69a032a6347805f274e6260322d42979`, `--with-openssl`, the old private prefix, and the original `pg_stat_statements` build and install. The retained log and matching stock binaries establish the stock LZ4 capability hold. [Stock binary pins](stock.sha256) · [Original capability receipt](../2026-11-23-slow-storage-transfer/frozen/native-postgres-capability.json) · [PostgreSQL source](https://ftp.postgresql.org/pub/source/v17.11/)

The private build uses those same PostgreSQL bytes and a statically linked LZ4 1.10.0 archive built from source SHA-256 `537512904744b35e232912055ccf8ec66d768639ff3abe5788d90d792ec5f48b`. The added configure option is `--with-lz4`. The build captures exact compiler and build-tool identities, ABI library paths and hashes, configure flags, and installed program and extension hashes. A stock logical export is loaded into fresh PGLZ and LZ4 clusters and read back with the stock psql client. The bridge exercises `pg_stat_statements`; the Apalis migration correction also builds and exercises `pgcrypto` and `hstore` from the same PostgreSQL archive. The controller uses a fresh prefix, runs the compiler and PostgreSQL as the unprivileged benchmark user, and leaves stock binaries and services in place. [LZ4 source](https://github.com/lz4/lz4/releases/tag/v1.10.0)

The historical transfer owner was `native-slow-storage-compiled-transfer-v1`. The successor controller `native-controller-complete-post-wal-queue` confirmed the absence of a duplicate and owns all host access, registration, idle checks, and releases. The old hold grants zero host work. Every request below is qualification only.

- [First sealed attempt](controller-packet.json) released exit 1 after matching all five stock binaries. The check treated a leading space printed by stock `pg_config` as a flag difference. It stopped before building or running diagnostics. [Readback and release](evidence/native/setup)
- [First recovery seal](recovery/controller-packet.json) tokenizes the configure readback and allows surrounding whitespace. The controller held it before registration after Darwin exposed Apalis's private-build extension dependency.
- [Extension-complete recovery](recovery-2/controller-packet.json) passed stock identity and configure checks, then released exit 1 when the Linux launcher could not locate `runuser`. It stopped before compilation and diagnostics. [Release and raw archive](evidence/native/recovery-2/setup)
- [Launcher recovery](recovery-3/controller-packet.json) binds both native failures and the held attempt. It uses the explicit Ubuntu `runuser` path and requests the raw device ID from `findmnt`. The workload and Apalis extension build carry forward. It released exit 0 with all eight diagnostic arms. [Controller release and raw evidence](evidence/native/recovery-3/setup)

## Darwin rehearsal

The real private build and logical bridge passed. Twelve full functional arms covered four stock PGLZ controls and all eight private-build engine and codec combinations. Every arm reconciled 512 canonical jobs, 1,024 independently committed application writes, and 512 terminal receipts. Each private LZ4 arm had 512 physically compressed rows. Each PGLZ arm had 512 uncompressed rows. Apalis migrated successfully after installing the two separately built contrib modules. River and Apalis were rebuilt from the retained locked sources. Mesh and Oban used their retained Darwin executables. These are functional checks on Darwin. [Functional replay and negative controls](evidence/local/functional.json)

The stored ledgers and physical readbacks regenerate the same summary, including deliberate all-uncompressed, mixed and corrupted-payload controls. [Replay archive](evidence/local/functional-replay.tar.gz)

## Linux qualification

All eight diagnostics replayed on PostgreSQL SHA-256 `3f273b467aa57364f603a6758de24f1937098bddfcd5ef8b65c90df9caaec4b8` on the original disk and boot. The stock and private bridges report GCC 13.3.0 from Ubuntu package `13.3.0-6ubuntu2~24.04.1`. Each LZ4 arm physically compressed all 512 payloads. Each PGLZ arm left all 512 uncompressed. Every arm verified its canonical payload hashes and the application and settlement transactions. Negative controls rejected corrupted and relabeled receipts for every engine. [Native replay](evidence/native/recovery-3/setup/independent-replay.json) · [Native negative controls](evidence/native/recovery-3/setup/codec-negative-controls.json)

## Evidence and timing gate

The setup scorer checks the original ledger, per-arm clocks, independent writes, and exact input. It also checks live PostgreSQL identity, TLS, role and database limits, durable server settings, stock payload DDL, and the applied compression. All 512 rows must be physically LZ4-compressed in each LZ4 arm. All 512 must be uncompressed in each PGLZ arm. Apalis is checked using the stored `bytea` column and decoded logical payload. River follows the previously proven export repair. The Linux adapter owns its stock `ledger.json` write. The Darwin branch explicitly writes the verified ledger when the Linux RSS sampler is unavailable. [Previous River export and scorer evidence](../2026-11-24-slow-external-compat-scorer-input/README.md)

The [matched timing inventory](recovery-3/matched-inventory.json) and [separate timing packet](recovery-3/timing-packet.json) bind the successful SETUP release and all eight replayed archive hashes. The timing request uses the five original orders with one warmup and measured arm per engine-policy cell in each order, for 80 arms total. Its replay checks the canonical timing release, rejects arm reuse, records PGLZ minus LZ4 within each engine, and reports Mesh minus River, Oban and Apalis within each block under each codec on both settlement clocks. It also reports first, 64th, and final first-application-commit landmarks. Historical DEFAULT and EXTERNAL estimates keep their own sources and identities.

The portable packet uses the existing in-checkout frozen archive and harness. The latest stage tar under `target/slow-lz4-isolated-qualification` retains both pinned source archives. Earlier stage seals remain recorded in their respective requests. The controller owns the inbox and journal. Do not invoke host-facing commands from an implementation thread.

```sh
p=tools/durable-jobs-bench/results/2026-11-25-slow-lz4-isolated-qualification
python3 -B "$p/recovery-3/recover.py" check
python3 -B "$p/rehearsal.py"
# After a successful canonical controller SETUP release:
python3 -B "$p/recovery-3/recover.py" replay-setup
python3 -B "$p/recovery-3/recover.py" seal-timing
# After the matched controller TIMING release:
python3 -B "$p/recovery-3/recover.py" replay-timing > "$p/evidence/native/matched-replay.json"
python3 -B "$p/report_timing.py" "$p/evidence/native/matched-replay.json"
```
