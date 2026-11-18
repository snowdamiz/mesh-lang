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

## Clean up after your task

Keep track of what you create outside the Git checkout. Before you finish,
delete everything you created there for the task, including temporary files,
test data, build outputs, logs, caches, and disposable checkouts. Clean up
after failed or canceled runs too.

Preserve pre-existing files, resources owned by others, user-requested
deliverables, and anything under an explicit hold. Report the path and reason
for anything you could not safely remove.
