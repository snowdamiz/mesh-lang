# Integrated enqueue native correctness

The exact product source `84e1cec9284b3c3193160a0ff9f3b56562eef075` passed
the PostgreSQL 17.11 compatibility and compiled effects gates on the
original Linux AMD64 Linode. Both gates exited 0. The wire check passed all
seven cases, covering malformed or missing rows, invalid receipts, failure
before commit, failure after command completion, and a lost commit reply.
The compiled effects suite verified the two application commits, exact
child reaping, recovery after an interruption, and cleanup on parent EOF.
[Successful raw proof](evidence/native/set-based-product-1173db55ef41/raw.tar.gz) ·
[Release](evidence/native/set-based-product-1173db55ef41/controller-release.json)

The retry kept the source archive and PostgreSQL executable pins. Cargo
used LLVM 21.1.8 for native linking, and the unprivileged proof read the
offline registry from a private copy. The compatibility extensions were
built from the pinned PostgreSQL source into the proof's isolated copy.
[Retry packet](request-private-cache.json) ·
[Raw archive hashes](evidence/native/set-based-product-1173db55ef41/SHA256SUMS)

Earlier attempts retain the full offline-fetch, LLVM-link and Cargo-cache
errors. This grant was for native correctness. [Native attempts](evidence/native/)
