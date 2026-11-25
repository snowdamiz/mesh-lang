# Provenance

The candidate `f3b3612d9`, its base `b3557273d`, and strict `955b356b5` are not
reachable from a branch. Both bundles keep them in Git. Their prerequisites
(`734416cdb` and `3828f20d2`) are on this branch.

    git fetch provenance/candidate-f3b3612d9.bundle HEAD   # then FETCH_HEAD = f3b3612d9
    git fetch provenance/strict-955b356b5.bundle HEAD      # then FETCH_HEAD = 955b356b5

`request.json` also pins five ignored `target/` files. The committed copies with the
same hashes are listed in [manifest.json](manifest.json). To replay from an export,
first copy each one back to its `target/` path. Then run
`python3 -B tools/durable-jobs-bench/results/2026-11-25-public-large-exact-prepare/controller.py check`.
