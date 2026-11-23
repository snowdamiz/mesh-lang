# November 23 native benchmark checkpoint

**Setup in progress.** SSH to `root@172.234.135.126:22` initially timed out before authentication. The saved client transcripts in [access/](access/) preserve the failures and the subsequent successful authentication after the firewall rule was saved. The host preflight records Ubuntu 24.04.5, four online x86-64 vCPUs, 7,941 MiB of RAM, and ext4 on `/dev/sda`. No official timing arms have run. The comparison and the Mesh optimization repeat remain open.

The frozen Mesh fixture calls `Jobs.start_effects`. At `dbcb6dbc1de3a6887fc3757c2b91f7974d74ed17`, `compiler/mesh-rt/src/jobs/mod.rs` and the worker dispatch require GNU Linux ARM64 for that path. The unchanged x86-64 build exercises the unsupported-platform return. Commit `3f69b150` ports the fenced effects path to AMD64. The matrix requires the port's native smoke before timing. [Native adapter handoff](../../native/README.md)

The offline matrix tests use all five frozen inputs and both reversed initial orders. They replay 100 synthetic measured arms and 60 warmup arms. They also replay both complete reversed controls, reject incomplete matrices, and verify the durable producer and settlement windows for a complete replay. The historical parity driver's 19 checks and 12 failure-driver tests passed. The repository's effect-fence proof also passed on a local disposable PostgreSQL 17.11 instance over TLS. It verified two independent commits, owner replay, cancellation, and expiry. These checks contribute zero native performance samples.

```sh
python3 -B tools/durable-jobs-bench/test_matrix.py -v
python3 -B scripts/jobs-parity.py --self-check
python3 -B -m unittest discover -s scripts/tests -p test_jobs_parity_fault.py
PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin PYTHONDONTWRITEBYTECODE=1 \
    bash scripts/verify-postgres-jobs.sh --effect-fence
```

To retry the authorized endpoint from the checkout root, run `python3 -B tools/durable-jobs-bench/run.py check-host --series 2026-11-23-native`. The command uses the dedicated SSH identity and the native owner's pinned `known_hosts` file. After the native setup and Mesh port are validated, fill `inventory.local.json` from the four adapter handoffs and follow the [controls and baseline commands](../../README.md#run-the-matrix).

The SSH transport test preserved a deliberately rejected invocation on the host and in the local [transport check](../2026-11-23-transport-check/README.md).

The coordinator holds official timing until River's replacement measures batch input inside the submit window and passes a native setup smoke. The older binary and translator receipts remain setup evidence. The AMD64 runtime proof and native runner builds use the shared `native.lock`. Apalis passed its Linux functional and sparse smokes, and Oban passed its Linux short smoke.
