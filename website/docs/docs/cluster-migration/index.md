---
title: Cluster Migration
description: Move from manual or primary/standby Mesh deployment to autonomous protocol two
---

# Cluster Migration

Migrate in reversible gates. Do not enable horizontal scaling until identity, protocol-two transport, durable continuity, admission, telemetry, routing, controller quorum, and driver observation are healthy.

## Compatibility matrix

| Cluster state | Protocol one peers | Protocol two peers | Autonomous scaling | Adaptive routing |
| --- | --- | --- | --- | --- |
| Manual protocol one | Supported | Supported | Off | Optional static fallback |
| Mixed version | Supported for data peers | Supported | Off | Off until every eligible peer advertises required capabilities |
| Autonomous | Controllers rejected if incompatible | Required | Available with quorum | Available for Ready compatible workers |

## Protocol capability timeline

| Release stage | Read compatibility | Write behavior | Rollback boundary |
| --- | --- | --- | --- |
| Protocol-one manual | Protocol one data frames | Protocol one only | Existing manual binaries |
| Transition | Protocol one and protocol two data frames | New identities and continuity are written only after capability negotiation | Fixed capacity, protocol-one placement; retain version-two state |
| Autonomous candidate | Protocol two plus retained protocol-one data decoder | Autonomous control stays disabled until every voter and eligible worker has the required capabilities | Pause automation and use fixed desired capacity |
| Autonomous current | Protocol two required for controllers, drivers, and autonomous workers | Fenced consensus, durable continuity, adaptive routing, and driver operations | Static/manual placement with protocol-two transport and durable state |

Protocol-one compatibility is a migration decoder, not an autonomous security mode. Its removal requires a separately announced compatibility release after retained fleet telemetry shows no protocol-one peers.

Protocol negotiation includes version, frame bounds, roles, and feature capabilities. An older peer is never allowed to interpret a new control frame silently. Runtime JSON schema 5 exposes `protocol_version`, `protocol_capabilities`, `autonomous_protocol_enabled`, and `protocol_disabled_reason` for every peer; treat an unknown schema as incompatible instead of guessing at a rollout gate.

## Migration sequence

1. Upgrade binaries while keeping manual placement and fixed scheduler capacity.
2. Configure stable node, boot, cluster, and application identities plus mutually authenticated TLS.
3. Enable protocol-two sessions and verify persistent multiplexed traffic, queue bounds, and operator queries.
4. Enable each node's private durable continuity store and verify restart hydration, snapshot resume, retention, and tombstones.
5. Enable admission and telemetry with local scheduler minimum equal to maximum.
6. Run adaptive routing in observation mode, then enable it first for safe reads.
7. Bootstrap an explicit odd controller voter set and verify majority commit and leader failover.
8. Configure the capacity driver in observe-only mode and reconcile all existing objects by labels.
9. Enable bounded scale-up while automatic scale-down remains disabled.
10. Run the Docker release proof and a production-driver staging proof.
11. Enable one-node-at-a-time scale-down and verify drain, transfer, re-replication, termination, and observed absence.

Before advancing from step 3, exercise the complete mixed-version path with all required capabilities: roll version one to version two one node at a time, confirm autonomous control remains disabled until every voter and eligible worker advertises protocol two, then roll back version two to version one while capacity is fixed. Retain version-two continuity and consensus state throughout the rollback. A successful version-two-only cluster is not evidence for this gate.

## Source and manifest changes

Cluster-wide policy belongs in `[cluster]` manifest sections. Handler replication remains source-first through `@cluster(N)` and `HTTP.clustered(N, handler)`, where `N` is the total copy count including the owner.

Keep existing `Node.start` and `Node.start_from_env` applications in manual mode until the manifest is ready. Existing environment settings remain migration aliases, but the validated embedded manifest is authoritative in autonomous mode.

## Primary/standby migration

Do not reinterpret a primary/standby promotion epoch as controller consensus. Introduce the controller quorum first, keep capacity fixed, synchronize node-local continuity stores, and verify fenced ownership generation before allowing more than one active ingress to place work.

## Rollback

Pause the autoscaler, set a safe fixed desired count, disable automatic scale-down, and switch routing to static/manual placement while retaining protocol-two transport and durable continuity. Revoking provider credentials stops new mutations but does not replace a committed pause. Do not discard version-two continuity or control state; export it before any binary downgrade.

If quorum is unavailable, restore a majority rather than forcing provider changes. If the driver is unhealthy, reconcile observed objects by labels before resuming. If mixed-version capability checks fail, keep service at fixed capacity until every voter and eligible worker is compatible.

Autonomous cluster-cookie generations and operator HMAC keys must each be at least 32 characters. Rotate credentials with overlapping verifier keyrings as described in [Cluster Operations](/docs/cluster-operations/#credential-lifecycle-and-rolling-rotation); do not combine a binary rollback with a signer-first credential rotation.

The local Fly fake-API conformance suite validates the driver contract but does not certify credentials, private networking, image access, or provider behavior. Complete and retain the credentialed Fly staging create/Ready/cordon/delete artifact before enabling Fly mutations in a production environment.
