# Provenance

The candidate `f3b3612d9`, its base `b3557273d`, and strict `955b356b5` are not
reachable from a branch. Both bundles keep them in Git. Their prerequisites
(`734416cdb` and `3828f20d2`) are on this branch.

    git fetch provenance/candidate-f3b3612d9.bundle HEAD   # then FETCH_HEAD = f3b3612d9
    git fetch provenance/strict-955b356b5.bundle HEAD      # then FETCH_HEAD = 955b356b5

The offline check reads three ignored inputs, all listed in [manifest.json](manifest.json):

- The five pinned `target/` files. They are committed here under the same hashes.
- The upstream checkout `target/2026-11-24-trusted-jobs-product-composition/source`,
  which is a tree of `b3557273d`.
- The upstream sealed intake's absolute `timing_command` path. That path belongs
  to the controller-owned FILE-session packet, which stays unchanged.

`verify-export.py` restores the first two into a fresh export. It remaps only the
original root prefix in memory, then runs the unmodified `controller.py check`.
It also checks that the bundles verify against the repository and that
`source.tar.gz` and `candidate.patch` match `f3b3612d9` byte for byte.

    git archive COMMIT | tar -x -C EXPORT
    python3 -I EXPORT/tools/durable-jobs-bench/results/2026-11-25-public-large-exact-prepare/provenance/verify-export.py \
        EXPORT REPO ORIGINAL_ROOT
