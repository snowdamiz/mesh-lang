---
title: Environment Variables
description: Every environment variable meshc, meshpkg, the installers, and the Mesh runtime read, with accepted values, defaults, and effects.
---

# Environment Variables

This page lists every environment variable that Mesh tooling or a compiled Mesh
program reads. Variables your own code reads through `Env.get` are yours, not
Mesh's, and are not listed.

In the tables, **program** means any executable built by `meshc`: its runtime
reads the variable in every process. **Controller**, **gateway**, and
**worker** mean a program whose `MESH_ROLES` includes that role. Set variables
before the process starts.

Rows marked **Secret** hold credentials. Load them from an owner-only file or a
secret store, keep them out of the repository and out of command lines, and
prefer the `--cookie-file` and `--operator-key-file` flags where a command
offers them.

## Compiler and tools

| Variable | Read by | Accepted values | Default | Effect |
| --- | --- | --- | --- | --- |
| `NO_COLOR` | `meshc build`, `meshc migrate`, `meshc test`, test binaries | Any value, even empty | Unset | Turns off colored diagnostics and test output. Color is also off when the output is not a terminal, or with `--no-color` or `--json`. See [Test Runner](/docs/tooling/#test-runner). |
| `MESH_RT_LIB_PATH` | `meshc` commands that link a program | Path to `libmesh_rt.a` (`mesh_rt.lib` on Windows MSVC) | The runtime in the `lib` directory beside the installed `meshc` (`~/.mesh/lib`), then a source checkout's Cargo target directory | Link against this runtime library. The file name must match the target's runtime name. Set but empty is an error. |
| `MESH_TEST_RT_LIB_PATH` | `meshc test` | Path to `libmesh_test_rt.a` (`mesh_test_rt.lib`) | Same search as above | The same override for the test runtime. |
| `CARGO_TARGET_DIR` | `meshc` | Directory | The first `target` directory found walking up from the `meshc` executable | Where a source-checkout `meshc` looks for `debug/` and `release/` runtime libraries (under `<triple>/` for `--target`). |
| `LLVM_SYS_211_PREFIX` | Source builds; `meshc` on Windows MSVC | LLVM 21 install prefix | `clang` from `PATH` | Locates LLVM 21 for a source build. On Windows MSVC, `meshc` links with `<prefix>\bin\clang.exe` and fails if that file is missing. See [Build from source](/docs/getting-started/#alternative-build-from-source). |
| `ANDROID_NDK_HOME`, `ANDROID_NDK_ROOT` | `meshc build --target *-linux-android` | Android NDK root | None | Required for Android targets; `ANDROID_NDK_HOME` wins. `meshc` uses the NDK's API 26 `clang` from `toolchains/llvm/prebuilt/*/bin`. |
| `MESH_BUILD_TRACE_PATH` | `meshc` | File path | Unset: no trace | Writes a JSON record of the build's last stage, target, runtime path, and error to this file. Use it to diagnose a failing build. |
| `DATABASE_URL` | `meshc migrate up`, `down`, `status` | PostgreSQL URL | None; required | **Secret.** The database migrations run against. `meshc migrate generate` does not need it. See [Database migrations](/docs/tooling/#database-migrations). |
| `HOME` | `meshc repl`, `meshpkg` | Directory | None | The REPL keeps history in `$HOME/.mesh_repl_history` and keeps none when `HOME` is unset, Windows included. On macOS and Linux, `meshpkg` stores its registry token in `$HOME/.mesh/credentials`. |

## Installers and updates

`install.sh` and `install.ps1` read these variables. `meshc update` and
`meshpkg update` pass the four `MESH_INSTALL_*` variables on to the installer
they run. See [Install the CLI tools](/docs/tooling/#install-the-cli-tools).

| Variable | Read by | Accepted values | Default | Effect |
| --- | --- | --- | --- | --- |
| `MESH_INSTALL_RELEASE_API_URL` | Installers | URL | GitHub's latest-release API for `hyperpush-org/mesh-lang` | Where the installer looks up the latest version when no version is given. |
| `MESH_INSTALL_RELEASE_BASE_URL` | Installers | URL | `https://github.com/hyperpush-org/mesh-lang/releases/download` | Base URL for `v<version>/<archive>` and `v<version>/SHA256SUMS`. |
| `MESH_INSTALL_DOWNLOAD_TIMEOUT_SEC` | Installers, `meshc update`, `meshpkg update` | Positive integer (seconds) | `120` | Timeout for each download. Any other value is ignored and the default used. |
| `MESH_INSTALL_STRICT_PROOF` | Installers | `1`, `true`, `yes`, `on` | Off | A missing `SHA256SUMS`, a missing entry, or a malformed checksum fails the install instead of printing a warning. `install.sh` also accepts `TRUE`, `YES`, and `ON`; `install.ps1` ignores case. |
| `MESH_UPDATE_INSTALLER_URL` | `meshc update`, `meshpkg update` | URL | `https://meshlang.dev/install.sh` (`install.ps1` on Windows) | The installer script the `update` commands download and run. Mesh runs whatever script this URL serves. |
| `HOME`, `USERPROFILE` | `install.sh`, `install.ps1` | Directory | None | The install root: `~/.mesh` from `install.sh`, `%USERPROFILE%\.mesh` from `install.ps1`. |
| `SHELL` | `install.sh` | Shell path | None | When it names `zsh`, the installer adds its `PATH` line to `~/.zshrc` even if that file does not exist yet. |
| `NO_COLOR` | Installers | Any non-empty value | Unset | Turns off colored installer messages. |

## Cluster CLI and proofs

See [Cluster operator commands](/docs/tooling/#cluster-operator-commands) and
[Proof commands](/docs/tooling/#proof-commands).

| Variable | Read by | Accepted values | Default | Effect |
| --- | --- | --- | --- | --- |
| `MESH_CLUSTER_COOKIE` | `meshc cluster` | Cookie keyring (see [Node bootstrap](#node-bootstrap)) | None; required unless `--cookie-file` is given | **Secret.** Authenticates the CLI to the target node. A blank value is an error. |
| `MESH_OPERATOR_KEY` | `meshc cluster` mutations | Comma-separated keys; the CLI signs with the first key of at least 32 characters | None; required unless `--operator-key-file` is given | **Secret.** Signs `autoscale`, `scale`, `drain`, and `cancel-drain` requests. See [Credential rotation](/docs/cluster-operations/#credential-lifecycle-and-rolling-rotation). |
| `MESH_PROOF_TIME_SCALE` | `meshc proof` | Integer, clamped to 1–10 | 3 with up to 4 CPUs, 2 with 5–8, 1 above that | Multiplies every proof deadline for a slow machine. A non-numeric value is ignored. |
| `FLY_API_TOKEN`, or the name given to `--token-env` | `meshc proof fly-driver-staging` | Fly API token | None; required | **Secret.** The Machines API token. Each `--worker-env` name is also read from the environment and passed to the Machine. |
| `MESH_FLY_ALLOW_CUSTOM_API_BASE_URL` | Fly capacity driver, as used by `meshc proof fly-driver-staging` | `1`, `true`, `yes`, `on` (any case) | Off | Allows an HTTPS `--api-base-url` other than `https://api.machines.dev` and sends the bearer token there. Never enable it around production credentials. See [Fly Machines driver](/docs/capacity-drivers/#fly-machines-driver). |

Against an autonomous cluster, `meshc cluster` also uses the node TLS and
signed-identity variables from [Autonomous identity and trust](#autonomous-identity-and-trust).
Give it an identity whose only role is `operator`.

## Node bootstrap

A program reads these variables when it calls `Node.start_from_env()`. See
[Recommended Environment Bootstrap](/docs/distributed/#recommended-environment-bootstrap)
and the [clustered example](/docs/getting-started/clustered-example/).

| Variable | Read by | Accepted values | Default | Effect |
| --- | --- | --- | --- | --- |
| `MESH_CLUSTER_COOKIE` | Program | Comma-separated keys; the first key signs, any key verifies | Unset: standalone mode | **Secret.** Setting it selects cluster mode. In autonomous mode, every key must be at least 32 characters or the node fails to start. |
| `MESH_DISCOVERY_SEED` | Program | DNS name | None; required in cluster mode | Resolved on every discovery interval; the node connects to each address at `MESH_CLUSTER_PORT`. A blank value is an error. |
| `MESH_CLUSTER_PORT` | Program | 1–65535 | `4370` | Cluster listener port and discovery dial port. An invalid value makes `Node.start_from_env()` return an error; an empty value means the default. |
| `MESH_NODE_NAME` | Program | `name@host:port`, or `name@[ipv6]:port` | Derived; see below | Explicit node identity. The port must equal `MESH_CLUSTER_PORT`. An invalid value makes `Node.start_from_env()` return an error. |
| `MESH_NODE_HOST` | Program | Host name or IP address | The system host name | The advertised host when the name comes from the host name: `<hostname>@<MESH_NODE_HOST>:<port>`. |
| `FLY_APP_NAME`, `FLY_REGION`, `FLY_MACHINE_ID`, `FLY_PRIVATE_IP` | Program | Set by Fly Machines | None | When `MESH_NODE_NAME` is unset and any of these is set, all four are required and the node name becomes `<app>-<region>-<machine>@[<private-ip>]:<port>`. |
| `MESH_DISCOVERY_INTERVAL_MS` | Program | Positive integer (ms) | `5000` | How often discovery resolves the seed. An invalid value disables discovery with a message on stderr; the node still starts. |
| `MESH_CONTINUITY_ROLE` | Program | `primary`, `standby` (any case) | `primary` | Continuity authority role for a primary/standby pair. Any other value stops the program: `Node.start_from_env()` returns an error naming it. |
| `MESH_CONTINUITY_PROMOTION_EPOCH` | Program | Non-negative integer | `0` | Starting promotion epoch. A value that is not a whole number stops the program the same way. |

The node name comes from `MESH_NODE_NAME` first, then the four `FLY_*`
variables, then the system host name with `MESH_NODE_HOST`. Setting
`MESH_DISCOVERY_SEED`, `MESH_NODE_NAME`, `MESH_NODE_HOST`, or any `FLY_*`
variable without `MESH_CLUSTER_COOKIE` makes `Node.start_from_env()` return an
error. Fly sets its variables on every Machine, so a program there needs the
cookie to start at all.

## Autonomous identity and trust

See [Autonomous Clusters](/docs/autonomous-clusters/) and
[Credential rotation](/docs/cluster-operations/#credential-lifecycle-and-rolling-rotation).

| Variable | Read by | Accepted values | Default | Effect |
| --- | --- | --- | --- | --- |
| `MESH_CLUSTER_MODE` | Program, `meshc cluster` | `autonomous` (any case) | Unset: manual mode unless the manifest selects autonomous mode | Requires mutual TLS, a signed identity, and 32-character cookie keys, and turns on readiness gates and, on a controller, the consensus quorum. A manifest that enables autonomous mode does the same without this variable. |
| `MESH_AUTONOMOUS_MODE` | Program | `1`, `true`, `on` | Unset | Legacy spelling of `MESH_CLUSTER_MODE=autonomous`, with the same effect. Use `MESH_CLUSTER_MODE`. |
| `MESH_CLUSTER_ID` | Program, `meshc cluster` | Non-empty string, at most 256 bytes on a controller | `mesh` for operator-control checks | Cluster identity. Stable node IDs and signed claims must be scoped `<cluster-id>/…`. An autonomous controller fails to start without it. |
| `MESH_STABLE_NODE_ID` | Program | `<cluster-id>/…`, at most 512 bytes on a controller | Required in autonomous mode; otherwise the node name | Identity that survives restarts. It must match the node's signed claim and, on a controller, its voter entry. It also names the default continuity database. |
| `MESH_ROLES` | Program, `meshc cluster` | Comma-separated `controller`, `gateway`, `worker`, `operator` | `gateway,worker` | The roles this node holds, in any case. `operator` is for CLI identities. |
| `MESH_CONTROLLER_VOTERS` | Program | Comma-separated voter entries (format below) | None; required on a controller | The fixed controller voter set. On a controller, a malformed entry, a duplicate, an even count above one, or a missing entry for itself makes startup fail. |
| `MESH_TLS_CA_DER_B64` | Program, `meshc cluster` | Comma-separated base64 DER certificates | None | Roots trusted for node mutual TLS. |
| `MESH_TLS_CERT_DER_B64` | Program, `meshc cluster` | Base64 DER certificate | None | This node's certificate. |
| `MESH_TLS_KEY_DER_B64` | Program, `meshc cluster` | Base64 PKCS#8 DER private key | None | **Secret.** This node's private key. |
| `MESH_NODE_IDENTITY_ENVELOPE_B64` | Program, `meshc cluster` | Base64 signed identity envelope | None | This node's signed claim: cluster, stable ID, advertised name, roles, and an expiry of at most 31 days. It is checked on every handshake. |
| `MESH_NODE_IDENTITY_VERIFY_KEYS_B64` | Program, `meshc cluster` | Comma-separated base64 Ed25519 public keys | None | Keys that verify peers' identity claims. |
| `MESH_CAPACITY_IDENTITY_SIGNING_KEY_DER_B64` | Controller running the Docker or Fly driver in process; `mesh-capacity-driver` | Base64 PKCS#8 Ed25519 private key | None; required when the worker template sets `MESH_CLUSTER_MODE=autonomous` | **Secret.** Signs 30-day identity envelopes for the workers the driver creates. |
| `MESH_APPLICATION_ID` | Program | String | `mesh-application` | Scopes `Idempotency-Key` replay. Every node of one application must use the same value. |

Each `MESH_CONTROLLER_VOTERS` entry is `<stable-node-id>|<name@host:port>`,
for example `prod/controller/c1|c1@10.0.0.10:4370`. A single-voter set lets its
controller become Ready with no peers.

Set the three `MESH_TLS_*` variables together. A partial or undecodable set
stops the node from starting. Without them, manual mode uses an ephemeral
certificate and autonomous mode refuses to start. Likewise, set
`MESH_NODE_IDENTITY_ENVELOPE_B64`, `MESH_NODE_IDENTITY_VERIFY_KEYS_B64`, and
`MESH_CLUSTER_ID` together: in manual mode, setting only some of them makes
every handshake fail, and autonomous mode requires all three.

## Readiness and lifecycle

`MESH_MIN_HEALTHY_PEERS` and `MESH_APPLICATION_READY` feed readiness gates,
which exist only in autonomous mode. See
[Readiness and routing](/docs/autonomous-clusters/#readiness-and-routing).

| Variable | Read by | Accepted values | Default | Effect |
| --- | --- | --- | --- | --- |
| `MESH_MIN_HEALTHY_PEERS` | Program | Non-negative integer | `1`; `0` for the controller of a single-voter set | Stable protocol-two peer sessions required before Ready. A non-numeric value is ignored. |
| `MESH_APPLICATION_READY` | Program | `false` or `0` hold the gate closed | Ready | Keeps the node out of Ready through the application-readiness gate. |
| `MESH_NODE_STATE` | Program | `provisioning`, `joining`, `warming`, `draining`, `terminating`, `removed`, `failed` | Computed from readiness | Forces the lifecycle state this node reports, and with it its routing eligibility. Other values are ignored. An operator drain still takes precedence. |
| `MESH_STARTUP_WORK_DELAY_MS` | Program | Positive integer (ms) | `2500` | How long replicated runtime-owned startup work stays pending before it dispatches. Other values are ignored. |
| `MESH_DESIRED_CAPACITY` | Program | 0–65535 | The observed membership count | The desired capacity shown in operator snapshots when no operator override exists. It only affects reporting. |
| `MESH_OPERATOR_AUDIT_LOG` | Controller | File path | Unset: no file | Appends one JSON line per operator control decision. Mesh creates the file with mode `0600` and syncs it on every write. |

## Storage

See [PostgreSQL and SQLite have different jobs](/docs/autonomous-clusters/#postgresql-and-sqlite-have-different-jobs).

| Variable | Read by | Accepted values | Default | Effect |
| --- | --- | --- | --- | --- |
| `MESH_CONTINUITY_DB` | Program | File path | With an autonomous `mesh.toml`: its continuity `path`, else `$MESH_DATA_DIR/continuity-<hash>.db`. Otherwise: no store | This node's private SQLite continuity store. It overrides the manifest, including `durable_continuity = false`. An empty value disables the store, which keeps an autonomous node out of Ready. |
| `MESH_DATA_DIR` | Program | Directory | `.mesh` in the working directory | Where the default continuity database is created. |
| `MESH_CONSENSUS_DB` | Controller | File path | `/tmp/mesh-control-plane.redb` | The controller's consensus log store. Point it at persistent storage. |
| `MESH_CONTINUITY_SNAPSHOT_CHUNK_BYTES` | Program | Integer from 128 to below 16 MiB | The manifest's `snapshot_chunk_bytes`, else 1 MiB | Chunk size for replica snapshot transfer. Values outside that range, or not numbers, are ignored. |
| `MESH_CONTINUITY_DURABILITY` | Program | `degraded` (any case); any other value means strict | The manifest's `durability`, else strict | `degraded` lets execution continue when the replica acknowledgement threshold is not met; strict rejects the request. |

## Scheduler

Any program reads these, clustered or not. An unset or unparsable value falls
back to the value built in from an autonomous `mesh.toml`, then to the default
shown. See
[Scaling behavior](/docs/autonomous-clusters/#scaling-behavior).

| Variable | Read by | Accepted values | Default | Effect |
| --- | --- | --- | --- | --- |
| `MESH_SCHEDULER_MIN_WORKERS` | Program | Positive integer | The number of CPUs | Scheduler threads active at startup. |
| `MESH_SCHEDULER_MAX_WORKERS` | Program | Positive integer | The minimum | The most scheduler threads the local autoscaler may activate. It runs only when the maximum exceeds the minimum. |
| `MESH_SCHEDULER_TARGET_RUNNABLE` | Program | Positive number | `1.0` | Runnable actors per active worker that count as pressure. |
| `MESH_SCHEDULER_TARGET_QUEUE_WAIT_MS` | Program | Positive integer (ms) | `25` | Queue-wait target for scaling up. |
| `MESH_SCHEDULER_SCALE_UP_WINDOW_MS` | Program | Positive integer (ms) | `10000` | How long pressure must last before adding a worker. |
| `MESH_SCHEDULER_SCALE_DOWN_WINDOW_MS` | Program | Integer (ms) greater than the scale-up window | `300000` | How long low load must last before retiring a worker. |
| `MESH_SCHEDULER_COOLDOWN_MS` | Program | Integer (ms) | `30000` | Minimum time between changes. |

A worker bound of zero, or a minimum above the maximum, silently gives a fixed
scheduler with one thread per CPU. An invalid target or window keeps the
scheduler at its minimum and prints
`mesh scheduler: local autoscaling configuration invalid; keeping minimum`.

## Admission and routing

An unset or unparsable value falls back to the value built in from an
autonomous `mesh.toml`, then to the default shown; no warning is printed. See
[Readiness and routing](/docs/autonomous-clusters/#readiness-and-routing).

| Variable | Read by | Accepted values | Default | Effect |
| --- | --- | --- | --- | --- |
| `MESH_MAX_QUEUED_PER_NODE` | Program | Positive integer | `512` | Queued HTTP connections before the server answers `503` with `Retry-After: 1`. It applies to every Mesh HTTP server. |
| `MESH_MAX_QUEUED_BYTES_PER_NODE` | Program | Positive integer (bytes; no unit suffix) | `67108864` (64 MiB) | Byte limit for the same queue. |
| `MESH_MAX_INFLIGHT_PER_NODE` | Program | Positive integer | `256` | Clustered requests executing at once on this node. |
| `MESH_MAX_CONTROL_INFLIGHT` | Program | Positive integer | `32` | Concurrent control-plane requests, admitted separately from application work. |
| `MESH_ROUTING_TARGET_INFLIGHT` | Program | Positive integer | `128` | In-flight requests that count as full load in pressure and routing. |
| `MESH_ROUTING_TARGET_QUEUE_WAIT_MS` | Program | Positive integer (ms) | `25` | Queue wait that counts as full load. |
| `MESH_LOAD_REPORT_INTERVAL_MS` | Program | Integer (ms), at least 25 | `500` | How often this node sends load reports. |
| `MESH_LOAD_REPORT_TTL_MS` | Program | Positive integer (ms) | `2000` | How long a peer's load report stays fresh. A node without a fresh report is not routed to, so a value not above the report interval is raised to twice the interval. |
| `MESH_RETRY_BUDGET_PERCENT` | Program | 0–100 | `10` | Share of remote dispatches that may be retried. |
| `MESH_ADAPTIVE_ROUTING` | Program | `1`, `true`, `on` or `0`, `false`, `off` (any case) | On when the manifest enables adaptive routing; otherwise off | Turns adaptive owner selection on or off. |
| `MESH_CAPACITY_UNITS` | Program | 1–65535 | Active scheduler workers | This node's relative capacity, used to weigh its load when choosing an owner. |
| `MESH_MEMORY_PRESSURE` | Program | Non-negative number | `0` | A fixed memory-pressure component for load reports, where `1.0` means at target. |
| `MESH_FAILURE_DOMAIN` | Program | String of at most 256 bytes | Empty | Continuity replicas prefer nodes outside the owner's failure domain. |
| `MESH_PRESSURE_EWMA_ALPHA` | Program | Number, clamped to 0.01–1.0 | `0.2` | Smoothing factor for the decision pressure this node reports. |

## Capacity drivers

A controller reads these when horizontal autoscaling is enabled. A missing or
invalid required value stops the controller at startup. See
[Capacity Drivers](/docs/capacity-drivers/).

| Variable | Read by | Accepted values | Default | Effect |
| --- | --- | --- | --- | --- |
| `FLY_API_TOKEN`, or the manifest's `token_env` | Controller (Fly driver) | Fly API token | None; required | **Secret.** The Machines API bearer token. |
| `MESH_CAPACITY_WORKER_ENV_ALLOWLIST` | Controller (Docker and Fly drivers) | Comma-separated variable names | None | Copies each named variable from the controller's environment into every managed worker, skipping unset names. Use it to pass values such as `DATABASE_URL` without writing them into `mesh.toml`. A name containing `=`, or a value containing a newline, stops the controller. |
| `MESH_CAPACITY_DOCKER_NETWORK` | Controller (Docker driver) | Network name | The manifest's `network` | Overrides the Docker network when non-blank. |
| `MESH_DOCKER_DRIVER_ENDPOINT` | Controller (Docker driver) | `host:port` | Unset: run the Docker CLI in process | Sends Docker operations to the external driver service. |
| `MESH_DOCKER_DRIVER_SERVER_NAME` | Controller | TLS server name | `docker-driver` | The name the service certificate must match. |
| `MESH_DOCKER_DRIVER_CLIENT_CERT_DER_B64` | Controller | Base64 DER certificate | None; required with an endpoint | The controller's client certificate. |
| `MESH_DOCKER_DRIVER_CLIENT_KEY_DER_B64` | Controller | Base64 PKCS#8 DER private key | None; required with an endpoint | **Secret.** The controller's client key. |
| `MESH_DOCKER_DRIVER_CA_DER_B64` | Controller, `mesh-capacity-driver` | Comma-separated base64 DER certificates | None; required | Roots for driver mutual TLS. |
| `MESH_DOCKER_DRIVER_SHARED_KEY` | Controller, `mesh-capacity-driver` | Comma-separated keys of at least 32 characters | None; required | **Secret.** Request HMAC: the first key signs, any key verifies. |
| `MESH_DOCKER_BINARY` | Controller (in-process Docker driver), `mesh-capacity-driver` | Path | `docker` | The Docker CLI to run. |
| `MESH_DOCKER_EXECUTION_PREFIX_JSON` | Controller (in-process Docker driver) | JSON array of strings | None | Arguments placed between the Docker binary and each subcommand, such as `["--context", "workers"]`. Invalid JSON stops the controller. |
| `MESH_DOCKER_ENV_HOST_DIRECTORY`, `MESH_DOCKER_ENV_DRIVER_DIRECTORY` | Controller (in-process Docker driver) | Directories | Both `mesh-capacity-driver` under the system temporary directory | Mesh writes each worker's env file, mode `0600`, under the host directory and passes the same file under the driver directory to `docker run --env-file`. Use them when the Docker CLI sees a different filesystem. Set both or neither. |

### Driver service

The `mesh-capacity-driver` service checks these at startup and exits with
status 1 when one is missing or invalid. It also reads
`MESH_DOCKER_DRIVER_CA_DER_B64`, `MESH_DOCKER_DRIVER_SHARED_KEY`,
`MESH_DOCKER_BINARY`, and `MESH_CAPACITY_IDENTITY_SIGNING_KEY_DER_B64` from the
tables above.

| Variable | Read by | Accepted values | Default | Effect |
| --- | --- | --- | --- | --- |
| `MESH_DOCKER_DRIVER_LISTEN` | `mesh-capacity-driver` | `host:port` | `0.0.0.0:7443` | Listen address. |
| `MESH_DOCKER_DRIVER_SERVER_CERT_DER_B64` | `mesh-capacity-driver` | Base64 DER certificate | None; required | The service certificate. |
| `MESH_DOCKER_DRIVER_SERVER_KEY_DER_B64` | `mesh-capacity-driver` | Base64 PKCS#8 DER private key | None; required | **Secret.** The service key. |
| `MESH_DOCKER_DRIVER_ALLOWED_CLUSTER` | `mesh-capacity-driver` | Cluster ID | None; required | The only cluster whose requests the service accepts. |
| `MESH_DOCKER_DRIVER_ALLOWED_POOL` | `mesh-capacity-driver` | Pool name | None; required | The only pool it serves. |
| `MESH_DOCKER_DRIVER_ALLOWED_IMAGE` | `mesh-capacity-driver` | Image reference | None; required | The only image it runs. |
| `MESH_DOCKER_DRIVER_ALLOWED_NETWORK` | `mesh-capacity-driver` | Network name, or empty for none | None; required | The only network it attaches workers to. |
| `MESH_DOCKER_DRIVER_ALLOWED_ENV_NAMES` | `mesh-capacity-driver` | Comma-separated unique names of letters, digits, and `_` | None; required | The exact set of environment names a worker template must carry. |
| `MESH_DOCKER_DRIVER_FAULTS` | `mesh-capacity-driver` | Comma-separated `ensure_response_loss_once`, `docker_api_timeout_once`, `unhealthy_new_worker_once` | Unset | Fault injection for the release proof. Never set it in production. |

## Debugging

| Variable | Read by | Accepted values | Default | Effect |
| --- | --- | --- | --- | --- |
| `MESH_GC_STRESS` | Program | Any value, even `0` | Unset | Every actor heap collects at every opportunity. It is very slow, and exists to expose values the collector cannot see. |

## Set by Mesh

Mesh sets these for its own child processes, or uses them only as internal
plumbing. Do not set them yourself.

- `MESH_TEST_QUIET` and `MESH_TEST_COLOR`: set by `meshc test` for test binaries.
- `MESH_CAPACITY_OPERATION_ID` and `MESH_CONTROL_TERM`: set by the Process driver on the workers it starts.
- `MESH_MEMBERSHIP_GENERATION`: copied into load reports.
- `MESH_PROOF_*` (except `MESH_PROOF_TIME_SCALE`), `CARGO_INCREMENTAL`, and `RUST_TEST_THREADS`: set by `meshc proof` for the processes it runs.

Capacity drivers also overwrite `MESH_ROLES` on every managed worker with the
manifest's managed roles. The Docker and Fly drivers set `MESH_STABLE_NODE_ID`,
and when the worker template sets `MESH_CLUSTER_MODE=autonomous`, also
`MESH_NODE_NAME` and a freshly signed `MESH_NODE_IDENTITY_ENVELOPE_B64`.
