# Replay the sealed recovery pilot from Git

The registered `controller-packet.json` has SHA-256 `b53ee2facf8e01ae8cff0904c01dfd207a1d1b196e2e8fe413e63dd5ec082122`. The candidate SQL lives in the sibling results directory. Its SHA-256 is `86a17110290c835a983cbdae9feb0a5007d397d8aab3300b2d018fda52cecbc9`. The stock definition comes from `compiler/mesh-rt/src/jobs/schema.sql` at `b3557273d72be3802bf189b284e7caaf5f3caa0f`. The stock definition, including its trailing newline, has SHA-256 `c5e348c3f324c1e0a7b7d38715be47ae557b1bcfd46eb63e4edeffc9c5046763`.

The included Git bundle contains the exact source commit. Its prerequisite, `734416cdbce2e557cb7dd7c4e2833ad7a4b5703d`, is in the handoff commit's ancestry. The archived hot 2×32 arm contains the compiled artifact manifest, the verifier's request, and its raw ledger. The manifest records compiler SHA-256 `e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19` and the source archive rooted at build base `dbcb6dbc1de3a6887fc3757c2b91f7974d74ed17`. The controller check reconciles that arm and checks the candidate against the bundled commit.

From the repository root, export the paths in `replay-inputs.sha256` at this handoff commit. Use Git's local object store as a read-only alternate so the scratch repository can resolve the bundle prerequisite.

```bash
set -euo pipefail
dir=tools/durable-jobs-bench/results/2026-11-25-empty-recovery-native
objects=$(git rev-parse --path-format=absolute --git-path objects)
scratch=$(mktemp -d)
trap 'python3 -c "import shutil, sys; shutil.rmtree(sys.argv[1])" "$scratch"' EXIT
{
    printf '%s\n' "$dir/replay-inputs.sha256"
    awk '{print $2}' "$dir/replay-inputs.sha256"
} | xargs git archive HEAD | tar -xf - -C "$scratch"
(
    cd "$scratch"
    shasum -a 256 -c "$dir/replay-inputs.sha256"
    git init -q
    export GIT_ALTERNATE_OBJECT_DIRECTORIES="$objects"
    git bundle verify "$dir/source-b3557273.bundle"
    git bundle unbundle "$dir/source-b3557273.bundle"
    python3 -B "$dir/controller.py" check
)
```

The check uses the historical hot-arm archive. Pilot dispatches, failures, and release evidence belong to the sole controller's separate native run.
