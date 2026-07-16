# Agent repository rules

This repository is self-contained. Keep compiler, runtime, tooling, examples,
documentation, installers, registry, package-site, and release-proof changes
inside this checkout.

## Verification

- Preserve unrelated user changes in a dirty worktree.
- Use the smallest focused test while iterating, then run the authoritative
  workspace and proof gates before claiming completion.
- Install the repo-owned pre-commit hook with
  `bash scripts/install-git-hooks.sh` when hook setup is requested.
- Never make release claims from stale test or proof output.

## Safety

- Do not commit secrets, generated credentials, private keys, or local database
  files.
- Do not use destructive Git commands to discard changes.
- Keep provider mutations behind the authenticated, fenced capacity-driver
  boundary.
