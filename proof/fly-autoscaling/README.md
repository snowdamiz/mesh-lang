# Fly autonomous autoscaling proof

This is the credentialed, provider-real companion to the mandatory local Docker proof. It creates an isolated one-hour topology in one Fly organization:

- three fixed controller Machines;
- two fixed gateway Machines;
- two fixed baseline worker Machines;
- zero to three policy-created worker Machines (two to five total workers);
- one Fly Managed Postgres cluster shared by every application process;
- one Fly-resident load generator and evidence collector with an encrypted volume.

The controller application alone receives the capacity-driver signing key and the Fly token needed by Mesh. The data application receives neither. The runner receives a token only for evidence collection and one fenced managed-worker-loss injection. Every created resource includes the twelve-digit proof run ID in its name or metadata.

Run:

```bash
proof/fly-autoscaling/provision.sh --duration-seconds 3600 --region iad
```

The command prints a `state_dir`. A monitor should invoke the following every ten minutes:

```bash
proof/fly-autoscaling/monitor-and-cleanup.sh <state_dir>
```

Before the runner finishes, the monitor writes a local provider/checkpoint observation and makes no mutations. At the terminal observation it retrieves the evidence, captures final provider state, and deletes exactly the three run-named apps, the runner volume, and the run-named Managed Postgres cluster. It refuses cleanup if any resource name fails its run-ID fence.

The unrelated `proof/fly-continuity-soak` application is outside this lifecycle and is never selected by these scripts.
