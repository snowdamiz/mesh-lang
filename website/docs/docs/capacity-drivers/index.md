---
title: Capacity Drivers
description: Provision process, Docker, and Fly capacity through Mesh's fenced driver contract
---

# Capacity Drivers

A capacity driver is the narrow authority boundary between Mesh desired state and an execution substrate. It validates configuration, observes managed capacity, ensures a node, begins drain, terminates a node, and looks up an operation.

Every mutation is idempotent and includes the cluster ID, operation ID, control term, desired revision, immutable template revision, and deadline. After any timeout, observe by those identities before retrying.

## Shared requirements

- Never interpolate a manifest value into a shell command.
- Restrict observation, adoption, and deletion to the configured cluster and pool.
- Adopt only an exact template and identity-label match.
- Treat repeated operation IDs as lookup of the existing result.
- Classify retryable, permanent, pending, succeeded, and unknown outcomes.
- Run provider calls outside transport and scheduler critical paths.
- Bound concurrency, queues, retries, response sizes, and deadlines.
- Read credentials from named environment variables or a protected external channel.
- Redact environment values, tokens, cookies, database URLs, and provider payload secrets.

## Process driver

The Process driver starts an argv array directly in a configured working directory. It is useful for one-host development and protocol testing. It does not prove container isolation or multi-host elasticity.

```toml
[cluster.capacity]
driver = "process"

[cluster.capacity.process]
command = ["./output"]
working_directory = "."
```

## Docker driver

The Docker driver creates and removes labeled worker containers from an immutable image. The driver must be the only proof component with Docker Engine authority; application workers must not receive the socket.

```toml
[cluster.capacity]
driver = "docker"

[cluster.capacity.docker]
image = "registry.example.com/app@sha256:..."
pool = "workers"
template_revision = "release-42"
network = "app-private"
env = ["DATABASE_URL", "PORT=8080", "MESH_ROLES=worker"]
```

Managed containers carry cluster, managed, pool, template, operation, term, and desired-revision labels. Create-response loss is handled by finding the exact labeled container. Removing an already absent container succeeds. A container with a mismatched cluster, pool, or template is never adopted or deleted.

An unrestricted Docker socket is host-root-equivalent. Use the dedicated mTLS driver service or a narrowly constrained socket proxy and rotate its credentials independently from application nodes.

The driver service accepts comma-separated CA roots in `MESH_DOCKER_DRIVER_CA_DER_B64` and comma-separated request keys in `MESH_DOCKER_DRIVER_SHARED_KEY`. Both sides sign with the first request key and verify every listed key, which permits the overlap-first rolling procedure in [Cluster Operations](/docs/cluster-operations/#credential-lifecycle-and-rolling-rotation).

The socket-bearing service also requires two local policy values:

```bash
MESH_DOCKER_DRIVER_ALLOWED_NETWORK=app-private
MESH_DOCKER_DRIVER_ALLOWED_ENV_NAMES=DATABASE_URL,PORT,MESH_ROLES
```

The network and exact sorted environment-name set must match every authenticated request. Values remain covered by mTLS/HMAC authentication and are never printed by `Debug`; constrain which controllers hold the driver key and put application-secret authorization at the secret source. The service refuses first-request trust-on-first-use for the network or environment shape.

## Fly Machines driver

The Fly driver uses the Machines HTTP API directly. It does not shell out to `flyctl`.

```toml
[cluster.capacity]
driver = "fly"

[cluster.capacity.fly]
app_name = "mesh-production"
token_env = "FLY_API_TOKEN"
image = "registry.fly.io/mesh-production@sha256:..."
region = "lax"
pool = "workers"
template_revision = "release-42"
cpu_kind = "shared"
cpus = 1
memory_mb = 256
```

The bearer token is read only from `token_env`. Production manifests pin the origin to `https://api.machines.dev`; an arbitrary manifest cannot redirect the token. A direct test-only driver can use another HTTPS origin only when the process owner explicitly sets `MESH_FLY_ALLOW_CUSTOM_API_BASE_URL`, which forwards the configured bearer token to that origin and must never be enabled around production credentials.

Machine metadata uses the same identity and fencing labels as other drivers. HTTP 408, 429, provider timeouts, and server errors are retryable with operation-scoped exponential backoff and full jitter; validation and authorization errors are permanent. The driver re-observes before creation, adopts exactly one metadata match after create-response loss or controller restart, and treats absence during deletion as success. It rate-limits provider mutations independently from retry timing.

## External driver service

When provider credentials should not enter controller processes, run the driver service behind mutually authenticated TLS. The channel authenticates the controller identity, verifies a bounded signed request, rejects replay, and applies an independent control budget. The external service still implements the same idempotent operation semantics.

## Certification checklist

A production driver is not certified until tests cover validation, credential redaction, exact-label adoption, create-response loss, timeouts, partial success, orphan handling, already-removed resources, leader failover, retry bounds, and destructive-call authorization. Run credential-free Fly conformance with `meshc proof fly-driver-conformance`.

The credentialed staging gate creates and deletes one Machine and requires an explicit acknowledgement:

```bash
FLY_API_TOKEN=... DATABASE_URL=... \
meshc proof fly-driver-staging \
  --app-name mesh-staging \
  --image registry.fly.io/mesh-staging@sha256:... \
  --cluster-id mesh-staging-cert \
  --template-revision release-42 \
  --worker-env DATABASE_URL \
  --confirm-create-and-delete
```

The Docker release proof certifies the local driver path. Fly cannot be called production-certified until the target staging app retains a passing create, Ready, cordon, delete, and observed-removal bundle.
