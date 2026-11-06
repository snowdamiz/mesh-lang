# Raft log storage and group routing

`DurableRaftLogStore<C>` provides redb log persistence for a Raft config with
`NodeId = u64` and `Entry = openraft::Entry<C>`. It implements OpenRaft's
`RaftLogReader<C>` and `RaftLogStorage<C>` traits. `open(path)` opens a log
file without creating or applying an application state machine.

Each group owns a separate database file. A future Jobs group also owns its
indexed state machine and deployment identity validation. The log store does
not bind a file to a deployment or infer a command schema. Its caller must
select the correct protected path and validate its persisted deployment/schema
identity before starting Raft. Opening an already open file fails through redb.

`DurableConsensusLogStore` is the compatibility alias for
`DurableRaftLogStore<MeshRaftConfig>`. `open_durable_consensus_store(path)` still
opens the control log and `DurableConsensusStateMachine` on one database.
Control state application, snapshots, table names, metadata keys, and JSON
representations are unchanged. Append acknowledges its flush only after the
redb transaction commits. Purge deletes the prefix and records its boundary
in the same transaction.

## Internal group registry

`dist::consensus::registry::raft_rpc_registry()` returns the process registry.
The registry keys are `RaftGroup::Control` and `RaftGroup::Jobs(deployment)`.
Deployment names must be nonblank and at most 256 bytes. Registration also
validates the cluster name, local node name, and nonzero local node ID. The
cluster and node ID must match the registered Raft instance.

The crate-private integration boundary consists of these operations.

- `register_jobs<C>(deployment, identity, raft, runtime)` registers a real
  `openraft::Raft<C>` and returns a `RaftRegistration`. It rejects an occupied
  group instead of replacing it. It accepts no callback or executable handler.
- `resolve<C>(peer, group, cluster_name, source_id, source_name, target_id)`
  returns the typed Raft handle and its Tokio runtime handle. It checks the
  authorized group, deployment, cluster, authenticated peer name, local node
  name, nonzero source ID, target ID, and registered config type. The future
  transport decodes its bounded RPC and calls the corresponding Raft method.
- `unregister(registration)` removes only the matching registration incarnation.
  A stale token, repeated removal, or a token from another registry cannot
  remove a replacement. Dropping a token alone does not unregister a group.

Resolution and removal are serialized by the registry lock. A request resolved
before removal may finish on its old Raft handle. It cannot switch to the new
incarnation. A lifecycle owner must stop admission, unregister, and drain its
already resolved work before releasing the old node's resources.

`AuthorizedRaftPeer` has private fields. Its only production constructor is the
control path after the existing session negotiates autonomous capability.
There is no production constructor for a Jobs authorization. Jobs registration
and typed lookup exist, but Jobs session authentication, RPC encoding, frame
budgets, and transport dispatch are not implemented by this unit. The session
integration must authorize a deployment independently of request bytes before
constructing that authorization. A remote selector is not an authorization.

The existing control frame handler always selects the Control registration.
Legacy envelopes omit `group` and retain their exact encoding. An explicit
`Jobs` group is rejected on the control handler. Control command commits and
runtime snapshots read only the Control registration. The existing production
factory retains its registration for the process lifetime.

## Focused verification

The unit tests use a concrete counter command config to exercise the generic
log independently of the control config. Both persist votes, committed indexes,
command entries, truncation, and purge boundaries through reopen. A fixed legacy
redb fixture checks the control vote, log, and snapshot bytes. The control store
also runs OpenRaft's storage conformance suite.

Registry tests route real Raft appends to separate durable control fixtures
under Control and two Jobs keys. These are test fixtures, not a Jobs state
machine. Tests check rejection before effects, separate state, duplicate
registration, stale and foreign unregister tokens, and a request held across
unregister and replacement with a channel barrier.

The focused commands are below. Run them with the checkout's assigned Cargo
target directory and build-resource limits.

```sh
cargo test -p mesh-rt --lib dist::consensus -- --test-threads=2
cargo test -p mesh-rt --test consensus_from_env --test control_plane -- --test-threads=1
```

These checks qualify the log and registry unit. Jobs integration, independent
review, full workspace gates, and process proofs on the integrated parent head
remain separate requirements.
