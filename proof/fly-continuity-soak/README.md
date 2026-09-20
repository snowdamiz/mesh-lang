# Fly continuity soak

This fixture runs the exact unshortened release gate:

```text
meshc proof continuity-soak --evidence-dir /data/evidence
```

It has no HTTP service. One `shared-cpu-1x` Machine runs the proof, and a one-gigabyte Fly Volume retains `/data/evidence/summary.json`, the exact command exit code, and the command log. After the proof finishes, the wrapper idles so automation can retrieve the evidence and stop the Machine. The restart policy is `never`, so a failed or interrupted run cannot silently restart its 24-hour clock.

Deploy from the repository root after creating an isolated app and a `mesh_soak_data` volume in `iad`:

```bash
fly deploy --app <isolated-soak-app> \
  --config proof/fly-continuity-soak/fly.toml \
  --remote-only .
```

Retrieve the final artifact before removing the Machine, volume, or app.
