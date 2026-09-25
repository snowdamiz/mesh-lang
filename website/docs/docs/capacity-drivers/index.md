---
title: Capacity Drivers
description: Provision process and Docker capacity through Mesh's fenced driver contract
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
env = ["PORT=8080"]
```

Each `env` entry is a literal `NAME=value` given to every worker; the manifest
is rejected when an entry has no `=`. Pass a controller's own variables, such
as a `DATABASE_URL` secret, through by name instead: list them, comma
separated, in `MESH_CAPACITY_WORKER_ENV_ALLOWLIST` on the controller, and each
one that is set is added with the controller's value. The driver always sets
`MESH_ROLES` to the managed roles, replacing any value from either source.

```bash
MESH_CAPACITY_WORKER_ENV_ALLOWLIST=DATABASE_URL
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

## External driver service

When provider credentials should not enter controller processes, run the driver service behind mutually authenticated TLS. The channel authenticates the controller identity, verifies a bounded signed request, rejects replay, and applies an independent control budget. The external service still implements the same idempotent operation semantics.

## Certification checklist

A production driver is not certified until tests cover validation, credential redaction, exact-label adoption, create-response loss, timeouts, partial success, orphan handling, already-removed resources, leader failover, retry bounds, and destructive-call authorization. The Docker release proof (`meshc proof docker-autoscaling`) certifies the Docker driver path.
