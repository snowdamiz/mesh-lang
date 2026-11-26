# Dense confirmed-grant launch candidate

This results archive preserves the private candidate against public `1e70f18d624a841740d3bbb3d0fd09b269257f68`. The candidate uses the existing split claim thread to register confirmed grants before the dispatcher receives them. Individual settlement stays on the dispatcher.

The same-binary PostgreSQL wire proof recorded peer handler entry while the first terminal ReadyForQuery was still held and both slots remained charged. The off control recorded a pending peer. The 17 untimed cases include pause, scale, stop, lease expiry, invalid and unknown grant receipts, ambiguous application and terminal receipts, and confirmed actor teardown. The local runner, teardown and local-control manifests bind those receipts to their fixture and binary hashes. [Implementation and proof](archive/README.md) · [Local evidence](archive/evidence/local) · [Source diff](archive/claim-lane.patch)

The controller requests and payload are byte-for-byte copies of the existing submission. The commands in the sealed requests refer to the original `target/` checkout. The [native stage](archive/native-stage.tar) and [setup](archive/setup.json) and [timing](archive/timing.json) requests are retained here for provenance. This package records the local proof and planned Linode comparison.

## Offline replay

Run the offline replay from this directory in a Git checkout or a clean export of the results commit.

```sh
python3 -B replay.py
```

The replay verifies the archived files, reapplies the exact patch to the pinned public source, runs the sealed checks for the original producer, fixture, payload and native peer inventories, and checks the held-receipt evidence and local manifests. It reads the earlier pinned results from the same checkout or export. The temporary replay layout is removed on exit. [Expected replay output](replay.json)

The earlier campaigns provide the [historical completion writer](../2026-11-23-ordinary-completion-writer/README.md), the [strict admission comparison](../2026-11-23-admission-v3/README.md), and the [public dense claim-to-entry audit](../2026-11-25-dense-claim-to-entry/README.md).
