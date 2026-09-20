# Docker autoscaling release proof

Run the repository-owned proof with:

```bash
cargo run -p meshc -- proof docker-autoscaling
```

After installing `meshc`, the equivalent command is `meshc proof docker-autoscaling`.

The proof requires Docker Engine, Docker Compose v2, OpenSSL, and enough local capacity for three controllers, two gateways, PostgreSQL, two baseline workers, the isolated driver service, load and evidence containers, and up to five Ready workers.

The command builds immutable local application and driver images, validates the resolved Compose configuration, creates a unique project, and waits on observed state transitions. It sends traffic through both Mesh gateways, waits for a converged policy-driven peak, completes a synchronized 1,000-request remote burst, creates workers through the Docker capacity driver, kills a worker, kills the consensus leader, removes load, drains surplus workers, validates PostgreSQL state, writes evidence, and cleans up.

PostgreSQL is the shared application data and final integrity oracle. Every Mesh node has its own private SQLite continuity file; no container shares a SQLite database.

Use `--keep-running` to retain the topology after evidence collection, `--evidence-dir <directory>` to choose the bundle location, or `--no-build` to reuse the local proof images. The default always attempts cleanup after collecting logs, including on failure.

The final output prints the evidence bundle directory and `PASS` or `FAIL`. `summary.json` is the machine-readable gate. The synchronized burst requires 1,000/1,000 successful unique remote executions and p99 latency at or below 6,000 ms. The injected-failure load separately uses a 10,000 ms p99 budget and a declared error-rate budget. A failed assertion or cleanup exits nonzero.

The dedicated driver service is the only container with Docker Engine access. It requires `MESH_DOCKER_DRIVER_ALLOWED_NETWORK` and `MESH_DOCKER_DRIVER_ALLOWED_ENV_NAMES`; every authenticated request must match the configured network and exact environment-name set before Docker authority is used. Unrestricted Docker socket access is host-root-equivalent and this topology is a proof fixture, not a production credential boundary. Production deployments should use a constrained socket proxy or provider API with least-privilege credentials.

When a proof fails, start with `summary.json`, then inspect the burst/load summaries, capacity snapshots, consensus snapshots, driver operations, container labels, database integrity result, redacted Compose log, and the retained redacted logs for each managed worker. Re-run with `--keep-running` only when live inspection is necessary.
