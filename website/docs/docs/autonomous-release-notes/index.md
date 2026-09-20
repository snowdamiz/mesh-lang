---
title: Autonomous Clustering Release Notes
description: Shipped behavior, compatibility window, gates, and rollout status for Mesh autonomous clusters
---

# Autonomous Clustering Release Notes

## Mesh 14 autonomous-cluster candidate

This release adds runtime-owned local scheduler elasticity, adaptive execution routing, durable replicated continuity, an embedded controller quorum, desired-capacity reconciliation, Process, Docker, and Fly capacity drivers, graceful drain, and the authenticated operator surface.

The PostgreSQL Todo deployment is the reference application: PostgreSQL is shared application state, while each Mesh node owns a separate SQLite continuity store. The one-command local release proof creates Docker workers from policy decisions, routes through two gateways, fails a worker and controller leader, drains back to minimum, checks PostgreSQL integrity, retains evidence, and cleans itself up.

Autonomous mode remains explicit in `[cluster]`; upgrading a manual cluster does not silently grant provider authority or enable destructive scaling. Controllers remain a fixed, operator-administered voter set. Worker autoscaling never changes quorum membership.

## Capability and compatibility timeline

| Stage | Protocol and storage contract | Default behavior |
| --- | --- | --- |
| Mesh 14.0 | Protocol-two sessions and version-two continuity ship; protocol one remains a manual data-peer decoder | Manual mode remains unchanged; autonomous mode requires explicit manifest policy and credentials |
| Mesh 14.x compatibility window | Mixed data peers are accepted only with autonomous features disabled; all controllers and eligible workers must advertise required capabilities before activation | Operators roll identity, transport, continuity, admission, routing, then capacity gates |
| Future deprecation release | Protocol-one removal may be announced after fleet evidence shows the decoder is unused | No removal occurs without separate release notes and a downgrade/export procedure |

Version-two continuity and consensus state must be retained during rollback. Rollback pauses automation, fixes desired capacity, disables scale-down, and returns placement to manual/static behavior; it does not downgrade or discard durable state in place.

## Release gates

The feature is release-ready only when all of these artifacts are current:

- the full Rust workspace test suite;
- deterministic model/property gates;
- performance results compared with the checked-in budget;
- chaos coverage for worker, controller, driver, snapshot, readiness, and orphan failures;
- a completed bounded-retention soak record;
- `meshc proof docker-autoscaling` and its passing evidence bundle;
- a provider-specific certification result before naming any non-Docker driver production-certified.

Docker proof success does not certify Fly credentials or a production network. The Fly driver contract and deterministic conformance tests ship here, but a deployment must retain its own staging certification before active provider scaling.

Current repository evidence for this candidate:

| Gate | Status |
| --- | --- |
| PostgreSQL-backed Docker proof | Passing: 36/36 assertions, including 1,000/1,000 synchronized remote requests with 5,307 ms p99 against a 6,000 ms budget |
| Deterministic performance budget | Passing for all checked-in schema-two metrics |
| Repeated chaos | Passing for five complete rotated rounds |
| Fly fake-API conformance | Passing for all 13 credential-free contract tests |
| Continuity smoke | Passing, explicitly non-release |
| Full 24-hour continuity soak | Required release artifact; a short smoke does not satisfy it |
| Credentialed Fly staging create/Ready/cordon/delete | Required before Fly is called production-certified; not implied by local conformance |

Do not infer public-GA or Fly-production certification from a candidate build while either of the last two provider/time-bound artifacts is absent.

## Security changes

Autonomous peers require mutually authenticated TLS and signed cluster-scoped identity claims. Operator and driver controls have separate authorization keys, replay bounds, rate limits, audit records, and redaction. Autonomous cluster-cookie generations and operator HMAC keys must each contain at least 32 characters. Runtime status schema 6 exposes every peer's negotiated protocol version, capability bits, autonomous enablement, disabled reason, bounded local runtime telemetry, continuity-store pressure, and per-peer send-buffer utilization so an operator can identify rollout and saturation blockers. TLS roots, identity verification keys, cluster cookies, operator keys, and driver request keys accept overlapping trust generations for rolling rotation; see [Cluster Operations](/docs/cluster-operations/#credential-lifecycle-and-rolling-rotation).

## Related guides

- [Autonomous Clusters](/docs/autonomous-clusters/) — configuration and application contract
- [Cluster Operations](/docs/cluster-operations/) — CLI, incidents, and credential rotation
- [Capacity Drivers](/docs/capacity-drivers/) — provider boundary and certification
- [Cluster Migration](/docs/cluster-migration/) — staged enablement and rollback
- [Distributed Proof](/docs/distributed-proof/) — mandatory local Docker evidence
