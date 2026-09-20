# Autonomous scaling decision log

This log records the decisions required to operate and reverse the autonomous clustering architecture.

## Embedded consensus

Owner role: Mesh control-plane maintainer.

Context: desired capacity, drain intent, manual overrides, and provider operations require one fenced writer that survives leader failure.

Decision: use OpenRaft with a durable embedded Redb store and an explicitly administered odd voter set. Autoscaling never changes voters.

Rejected: DNS ordering, primary environment flags, and last-write-wins broadcasts do not provide majority authority or fencing.

Consequences and reversal: controller state is local and replicated; operation terms come from consensus. Pause automation and retain manual membership to reverse without deleting the log.

Reversal cost: medium. Operators must freeze automatic changes, preserve/export the replicated log, and migrate desired state to an explicitly administered control path.

## Continuity storage

Owner role: Mesh continuity-storage maintainer.

Context: request continuity must survive restart without becoming a shared application database.

Decision: use one private SQLite store per node with runtime-owned migrations, retention, tombstones, checksummed snapshots, and protocol replication. PostgreSQL is reserved for shared application data.

Rejected: an unbounded process map loses restart durability; a shared SQLite file is not a multi-node application database; making PostgreSQL part of the control path couples recovery to application availability.

Consequences and reversal: each node needs durable local storage and catch-up before Ready. Export version-two records before downgrade; never silently discard them.

Reversal cost: high. A replacement store needs schema migration, replay/tombstone equivalence, snapshot compatibility, and retained-state export evidence.

## Protocol-two transport

Owner role: Mesh distribution-transport maintainer.

Context: per-request TLS connections and synchronous accept-path waits do not scale and can starve control traffic.

Decision: keep persistent authenticated peer sessions with capability negotiation, correlation IDs, independent reader and writer loops, and bounded control, application, and snapshot queues.

Rejected: one connection per request and one unclassified outbound queue.

Consequences and reversal: autonomous mode requires protocol two. Manual mode may fall back to protocol one during the compatibility window.

Reversal cost: medium. Automation must be paused and capacity fixed before eligible nodes are rolled back to protocol one; version-two state remains retained.

## Production capacity provider

Owner role: Mesh capacity-driver maintainer.

Context: the first production provider needs an idempotent API, immutable images, private networking, and metadata lookup after response loss.

Decision: implement Fly Machines through its HTTP API behind the common driver contract. Tokens are read from a named environment variable.

Rejected: shelling out to `flyctl` and a universal cloud abstraction.

Consequences and reversal: certify against a fake API and staging account. Revoke the token and pause automation to reverse.

Reversal cost: low for disabling Fly authority, high for replacing the provider. Revocation is immediate; a new production provider requires the full driver conformance and staging lifecycle gates.

## Docker authority boundary

Owner role: Mesh capacity-driver security maintainer.

Context: the mandatory local proof must create real containers without giving application workers host-root authority.

Decision: put Docker access in a dedicated mutually authenticated driver service and restrict all observation and deletion by managed cluster and pool labels.

Rejected: mounting the Docker socket into workers or using `docker compose scale` as the policy engine.

Consequences and reversal: the proof driver remains security-sensitive. Stop the service and remove its credentials to revoke authority.

Reversal cost: low. Disabling the isolated service removes Docker authority but also disables local horizontal reconciliation until another constrained driver is configured.

## Durability default

Owner role: Mesh continuity-storage maintainer.

Context: executing before the record acknowledgement threshold weakens recovery guarantees invisibly.

Decision: strict durability is the default; degraded progress requires explicit configuration and operator-visible metadata.

Rejected: best-effort replication presented as normal completion.

Consequences and reversal: strict mode may reject during replica shortage. Opt into degraded mode only with an accepted loss model.

Reversal cost: low to change an explicit deployment policy, but high to change the product default because documentation, failure semantics, and recovery evidence must be revised together.

## Maximum replica count

Owner role: Mesh compiler/runtime contract maintainer.

Context: replica metadata, acknowledgement fan-out, and failure-domain selection need a bounded public contract.

Decision: compiler and runtime enforce the same documented maximum, with counts defined as owner plus record replicas.

Rejected: accepting arbitrary source counts and failing later with a one-standby runtime limit.

Consequences and reversal: raising the maximum requires transport, storage, performance, and compatibility evidence; lowering it requires a migration diagnostic.

Reversal cost: high. The accepted source contract, generated metadata, acknowledgement fan-out, storage schema, CLI, and rolling compatibility matrix must move in lockstep.

## Canonical HTTP body hashing

Owner role: Mesh HTTP identity maintainer.

Context: an idempotency key must reject semantically different reuse without depending on gateway-local data.

Decision: hash raw body bytes plus method, stable route identity, normalized parameters, selected semantic headers, and authenticated tenant scope. No implicit JSON normalization is applied.

Rejected: raw URL hashing, arrival metadata, trace IDs, and undocumented body rewriting.

Consequences and reversal: changing normalization changes operation identity and therefore requires a versioned compatibility decoder.

Reversal cost: high. Existing idempotency keys cannot be reinterpreted safely without versioned hashes, a compatibility decoder, and collision/replay migration tests.

## Retention defaults

Owner role: Mesh continuity-storage maintainer.

Context: replay and delayed replicas need terminal history, but state must reach steady size.

Decision: retain terminal records for 24 hours and tombstones for 48 hours by default, with record and disk bounds plus bounded compaction.

Rejected: unbounded retention and active-record eviction under pressure.

Consequences and reversal: tune windows to the maximum retry and replication lag; shortening them requires proving delayed updates cannot resurrect records.

Reversal cost: medium. Configuration is reversible, but default changes require a full bounded-retention soak and delayed-replica/tombstone evidence.

## Gateway deployment

Owner role: Mesh routing and ingress maintainer.

Context: a public address still selects a TCP ingress, while Mesh selects the execution worker.

Decision: every application ingress and the optional gateway-only role use the same bounded routing engine. Public DNS or a platform address remains outside execution placement.

Rejected: sticky sessions, client-visible worker topology, and claiming Mesh is an authoritative global anycast service.

Consequences and reversal: gateways need peer connectivity and current load reports. A deployment can return to application-node ingress without changing handler semantics.

Reversal cost: low. Public ingress can move back to application nodes while retaining the same routing engine, handler identities, and continuity state.

## Compatibility window

Owner role: Mesh protocol-compatibility maintainer.

Context: existing manual clusters and environment aliases cannot change atomically.

Decision: protocol one remains available in manual mode for a documented transition, and old environment values remain aliases. Autonomous features stay disabled until every voter and eligible worker advertises protocol-two capabilities.

Rejected: silently enabling partial autonomous behavior in mixed-version clusters.

Consequences and reversal: rollback keeps fixed capacity and manual placement while preserving version-two state for a future upgrade.

Reversal cost: medium. Automatic actions must remain disabled until capability convergence, and removing protocol-one support requires a separately announced compatibility release.
