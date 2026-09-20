---
title: Cluster Operations
description: Inspect and operate an autonomous Mesh cluster safely
---

# Cluster Operations

This runbook is for an operator diagnosing or changing an autonomous cluster. Use a mutually authenticated operator identity and a target controller address. Runtime JSON schema 6 is current; automation should reject unsupported schema versions. Schema 6 adds bounded local scheduler, mailbox, HTTP latency, remote-dispatch, process-resource, capacity-driver, peer send-buffer, and continuity-store telemetry while preserving schema-5 read compatibility.

## Start with read-only state

```bash
meshc cluster status controller@10.0.0.10:4370 --json
meshc cluster capacity controller@10.0.0.10:4370 --json
meshc cluster pressure controller@10.0.0.10:4370 --json
meshc cluster routing controller@10.0.0.10:4370 --json
meshc cluster scaling controller@10.0.0.10:4370 --json
meshc cluster events controller@10.0.0.10:4370 --json
```

Distinguish desired, observed, Ready, and draining counts. Driver acceptance is not proof that a node exists or is Ready. A healthy process is not necessarily routing-eligible. A managed provider object that reports Ready but never joins authenticated Mesh membership is retained for the configured join/drain grace, then fenced, removed without a continuity drain, and replaced because it could not have owned admitted Mesh work.

The scaling snapshot also reports `observe_only`, `automatic_scale_up`, and `automatic_scale_down`. Each node exposes `protocol_version`, `protocol_capabilities`, `autonomous_protocol_enabled`, and `protocol_disabled_reason`; use these fields to identify the voter or worker blocking protocol-two activation. In observe-only mode, a recommendation is evidence—not a committed desired-capacity change or provider mutation.

Use `cluster explain` with a request key when investigating placement or rejection. Use `cluster continuity` to inspect the current attempt, owner, record replicas, durability, response retention, and terminal state.

## Pause before uncertain intervention

```bash
meshc cluster autoscale pause controller@10.0.0.10:4370 \
  --reason "investigating provider errors"
```

Pause prevents new automatic desired-capacity changes. It does not stop traffic, cancel an active termination, or change the current desired count. Confirm the committed pause in `cluster scaling` and `cluster events`.

Resume only after telemetry is complete, the controller quorum is stable, and the capacity driver can observe by cluster and operation labels:

```bash
meshc cluster autoscale resume controller@10.0.0.10:4370 \
  --reason "provider observation restored"
```

## Manual desired capacity

An authenticated manual override is committed through the same fenced control log as automatic decisions:

```bash
meshc cluster scale controller@10.0.0.10:4370 6 \
  --reason "planned event capacity"
```

The requested count must remain inside policy bounds. Observe provider state and Mesh readiness separately. Do not retry by inventing another operation ID; the reconciler re-observes the committed desired revision.

## Drain and cancellation

```bash
meshc cluster drain controller@10.0.0.10:4370 <node-id> \
  --reason "host maintenance"
```

Confirm that the node is Draining and routing-ineligible, active requests are decreasing or transferring, required record copies are re-replicated, and no unique capability or voter responsibility remains. Termination is not safe until every drain gate passes.

Cancel before termination if load rebounds or maintenance is abandoned:

```bash
meshc cluster cancel-drain controller@10.0.0.10:4370 <node-id> \
  --reason "load rebound"
```

Once termination begins, provision replacement capacity rather than assuming the instance can be resurrected safely.

If a drain exceeds `drain_timeout`, the default `forced_termination = "never"` leaves it Draining with `drain_timeout_manual_intervention_required`. Inspect active work and replica safety before acting. With the explicit `after_drain_timeout` policy, Mesh may fence and abandon active work only when the node holds no required or sole continuity copy and the draining membership generation is acknowledged; the event is labeled `forced_termination_after_drain_timeout` and is never reported as graceful completion.

## Failure triage

### Worker loss

Verify that its load report expires, routing excludes it, ownership is fenced, only safe work is recovered, and desired capacity is reconciled. Inspect continuity errors before forcing another node removal.

### Controller leader loss

The remaining majority should elect a leader with a higher term. Compare committed desired revision and driver operation IDs before and after election. No operation ID may identify more than one capacity object.

### Controller quorum loss

Scaling and destructive calls must freeze. Keep serving through already Ready nodes, restore a majority, and avoid manual provider mutations that Mesh cannot reconcile by its labels.

### Missing telemetry

A stale worker becomes routing-ineligible. Scale-down must remain frozen; “missing” is not “zero.” Check peer connectivity, protocol capability, load-report sequence, and control-queue health.

### Driver timeout or partial success

Observe provider objects by cluster, pool, template, operation, term, and desired-revision labels before retrying. Adopt one exact match. Quarantine mismatched orphans. Absence during deletion is success.

### Continuity pressure

Do not remove the only live copy of active state. Inspect pending records, replica acknowledgements, synchronization lag, disk limit, and compaction lag. Restore a replica or cancel the drain before destructive action.

### PostgreSQL outage

Database-backed routes should fail readiness or return an application failure. Mesh controller and continuity state remain independent. Restore PostgreSQL, verify migrations, and then allow application readiness to recover.

## Audit and secret handling

Every accepted or rejected control records a bounded actor/action/reason/sequence diagnostic; destructive actions additionally record term, desired revision, operation ID, and outcome. Rejection records never retain a signature. Logs and events must not contain cookies, TLS private keys, provider tokens, database URLs, request bodies, or idempotency keys. Treat unrestricted Docker socket access as host-root authority.

## Credential lifecycle and rolling rotation

Autonomous channels use overlapping trust keyrings so credentials can rotate with a rolling node restart instead of a simultaneous full-cluster restart. Comma-separated values are ordered: a sender signs with the first key and a receiver accepts any key.

| Credential | Configuration | Rotation behavior |
| --- | --- | --- |
| Node TLS roots | `MESH_TLS_CA_DER_B64` | One or more base64 DER roots; the local certificate and private key remain single-valued |
| Signed node claims | `MESH_NODE_IDENTITY_VERIFY_KEYS_B64` | One or more Ed25519 public keys; claims bind cluster, stable node ID, advertised name, roles, and expiry |
| Cluster-cookie compatibility HMAC | `MESH_CLUSTER_COOKIE` | Every generation must be at least 32 characters. First key signs the handshake; every listed key verifies. |
| Operator control HMAC | `MESH_OPERATOR_KEY` or owner-only `--operator-key-file PATH` | Every retained key must be at least 32 characters. The CLI signs with its first key; controllers verify every configured key. Literal secret argv flags are intentionally unsupported. |
| Driver TLS roots | `MESH_DOCKER_DRIVER_CA_DER_B64` | One or more base64 DER roots for both client and service verification |
| Driver request HMAC | `MESH_DOCKER_DRIVER_SHARED_KEY` | Each side signs with its first key and verifies every listed key |

Use this order:

1. Add the new CA or verification/HMAC key after the old key on every verifier and roll nodes while old credentials still sign.
2. Confirm all voters, gateways, workers, operators, and the driver trust both generations.
3. Put the new key first and issue new certificates or signed identity envelopes; roll one non-voter at a time, then one controller voter at a time while preserving a majority.
4. Confirm protocol-two sessions, quorum, operator controls, and driver observation across mixed generations.
5. Remove the old key and roll again. Revoke the old issuer or secret only after no retained process uses it.

Pause autoscaling before rotating controller or driver authority. Never rotate by changing only the signer first: peers that do not yet trust the new generation will correctly reject it. Identity claims are deliberately short-lived and cannot exceed 31 days; issue replacements before expiry.

## Evidence to retain

For an incident, retain versioned status snapshots, controller terms, desired revisions, decisions, driver operations, provider object labels, routing counts, continuity samples, redacted logs, and database integrity results. Record wall-clock time for correlation, but use monotonic deadlines for automated decisions.
