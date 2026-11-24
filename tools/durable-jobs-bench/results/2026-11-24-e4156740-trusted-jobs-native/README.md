# e4156740 Linux Jobs proof checkpoint

Parked offline at the public campaign pause. The source archive and proof
scripts are checksummed. Controller intake JSON, dispatch and native receipts
remain pending. The existing reservations and slow-cap71 hold stay in the
controller ledger. This proof contributes zero Jobs measurements. The existing
`b3557273` scores retain their original source and ELF binding.

- Product commit `e4156740c7409156409cd2f792f4875153d71040`, tree
  `00331956a38ebe4c28f0d601d0b51bd0a4a600e0`.
- `source.tar.gz` SHA-256
  `2a0bc0a5bba693c79e92fca0fd288b7c6d383a79cccf86d70372d521f8ae117a`.
  `pack.py` reads tracked Git inputs from that commit and appends the separate
  proof overlay. A second archive matched byte for byte. Key build, test and
  grant inputs were compared directly with Git. The eight frozen harness hashes
  match the retained `bulk-enqueue-ce5e551d079e.tar.gz` members.
- The Jobs driver rebuilds the exact product runtime. It pins the retained
  compiler `e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19`
  and PostgreSQL 17.11. `compiler-input-diff.txt` records runtime and test drift
  since the compiler source revision `3828f20d`. The real handler and two
  application writes reuse the archived fixture source. Only the runner options
  were narrowed. The callback witness adds the existing C teardown hook.
- The Jobs proof checks denial before the worker grant, a committed short
  set-based grant, a mixed small/large fallback and checked cursor grant,
  withheld claim receipts, idle application probes, distinct producer,
  application and individual terminal XIDs, ambiguous claim recovery, one
  committed application write response cut, and held/released or forcibly killed
  actor teardown. Six cases passed in the untimed macOS rehearsal on
  `b3557273` inputs. The local link produced a Mach-O. Its manifest step could
  not find workstation `rustc`. `rehearsal/identity.json` records this and the
  hashes. Linux qualification against the new runtime remains open.

## Resume after explicit release from the pause

From the repository root, verify the parked files:

```sh
(cd tools/durable-jobs-bench/results/2026-11-24-e4156740-trusted-jobs-native && shasum -a 256 -c SHA256SUMS)
```

Start with the grant and archive pattern in
`target/linode-jobs-stepchange/trusted-product-setup.py`. Finish a dedicated
controller intake wrapper and two separate request JSON files. The controller
must hold its local `.timing.lock` and an active exclusive SETUP grant for the
specific request ID before using the corresponding native argv. The native
script acquires both host fences itself. Feed the exact `source.tar.gz` as
stdin. These are the two planned native invocations for the wrapper:

```sh
bash native.sh jobs trusted-e4156740-jobs-2a0bc0a5bba6 e4156740c7409156409cd2f792f4875153d71040 2a0bc0a5bba693c79e92fca0fd288b7c6d383a79cccf86d70372d521f8ae117a
bash native.sh standalone trusted-e4156740-standalone-2a0bc0a5bba6 e4156740c7409156409cd2f792f4875153d71040 2a0bc0a5bba693c79e92fca0fd288b7c6d383a79cccf86d70372d521f8ae117a
```

Request the standalone slot separately from Jobs after resumption and outside
all official Jobs timings. It builds the exact commit compiler, records its
hash and ELF identity, runs only
`the_performance_gate_measures_and_reports`, and verifies one test passed and
that the compiler hash stayed fixed. Its output belongs to standalone test
evidence. Never run this command on the workstation.

For both modes, archive `/var/lib/durable-jobs-bench/native/runs/<attempt>`
through the controller on success or failure. Record the grant, actual exit,
raw archive hash and evidence status. For Jobs, verify both manifests against
the commit and archive, compare the runtime hashes, and require all six
`semantic-root/semantic/*/result.json` receipts. For the standalone request,
require `native.exit`, `compiler.sha256`, `test-inputs.sha256` and the exact
single-test output. Durable ELF copies remain under
`/var/lib/durable-jobs-bench/native/source-<attempt>/artifacts`.
