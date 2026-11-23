# Native dense worker cost

**No worker candidate qualified.** Mesh's full drain took 6.91% longer under observation, above the 5% screen. The profiled arm attributed 0.108 CPU-ms/job of named file-backed faults to a Mesh instruction. Even the sum across all named file-backed callers was 0.270 CPU-ms/job, below the 0.50 CPU-ms/job entry criterion. The capture did not identify file offsets for relocation. Unknown backing accounted for another 0.559 sampled CPU-ms/job and remains unresolved. [Worker pricing](recovered/pricing.json)

## Capture and lock handoff

The six diagnostic arms used the stock Linode artifacts from the baseline at `069c9c68612f0435e4885054288a5951a96cf9fc`. Each arm reconciled 1,024 jobs, 2,048 ordered application commits, and final settlement. PostgreSQL 17.11 and the runners used CPUs 0 and 1. The observer ran on CPU 2. The stock Mesh ELF hash was `49276df3d039e1ef9bfd07267367e3ec6996fe4b95bebd645e88414279b1b612`. The capture preserved the pinned compiler output, adapter code, TLS, durability, and workload. [Original stock profile](../2026-11-23-dense-diagnostics/README.md), [frozen pins](replay/pinned.json)

The initial reservation, `worker-182105z`, completed the trace-off Mesh and Oban controls. Their post-commit drains took 5.275 s and 1.439 s. Mesh trace-on failed before the cohort ran because the original collector called the nonexistent Python `time.clock_getcpuclockid`. The launcher exported the partial receipt and removed its host scratch. [Initial raw receipt](initial/evidence.tar.gz), [reduction](initial/reduced.json)

The repair uses libc's `clock_getcpuclockid` and checks the bound Linux pidfd before and after the read. An unavailable or exited PID contributes no clock reading. Five focused local tests exercised the clock binding, errors, and PID identity checks through substitute Linux calls. Docker Desktop could not start. The recovered dependency archive changes only `native.py`. [Repair](replay/runner-clock.patch), [input comparison](replay/seal-check.log), [local proof](replay/local-proof.log)

The recovery reservation, `worker-recovery-185640z`, ran all six arms in 104 s including export and cleanup. The launcher exited zero and released the local timing lock, remote active-arm lock, and remote native lock at 18:58:47 UTC. Both receipts retain the original empty cleanup logs. [Published lock handoff](lock-handoff.txt), [recovered raw receipt](recovered/evidence.tar.gz)

## Perturbation screen

The full drain spans the producer commit through the last terminal commit. The middle drain spans settlement ranks 103 through 922. The screen allows a 5% spread between trace-off controls and a 5% shift from their midpoint under observation.

| Engine and window | Trace-off controls, ms | Trace-on, ms | Trace-off spread | Trace-on shift | Screen |
| --- | ---: | ---: | ---: | ---: | --- |
| Mesh full drain | 5,157.845, 5,280.498 | 5,579.558 | 2.35% | +6.91% | Fail |
| Mesh middle drain | 4,021.599, 4,099.016 | 4,238.930 | 1.91% | +4.40% | Pass |
| Oban full drain | 1,468.248, 1,479.006 | 1,629.903 | 0.73% | +10.60% | Fail |
| Oban middle drain | 1,127.738, 1,071.257 | 1,140.039 | 5.14% | +3.69% | Fail |

## CPU observed in the Mesh trace-on arm

The reducer joined all 1,024 fresh worker PIDs to their exact consuming waits and ordered application and terminal XIDs. The waits measured 3.414 CPU-ms/job of worker lifetime CPU, including 2.723 system CPU-ms/job. Median minor faults were 308 per worker, with zero major faults. The lifecycle trace matched all 1,024 execs and leader exits. These measurements describe the observed arm. [Recovered reduction](recovered/reduced.json)

| Observed component | CPU-ms/job | Scope |
| --- | ---: | --- |
| Jobs PostgreSQL | 2.840 to 2.902 | Absolute CPU bounds across 32 stable ready-snapshot PIDs |
| Fenced application PostgreSQL | 1.774 to 1.781 | Absolute CPU bounds across 32 stable ready-snapshot PIDs |
| Runner parents | 1.718 to 1.739 | Absolute CPU bounds across eight stable process clocks |
| Kernel exec and user-space dynamic loader | 0.589 | 120 disjoint pre-handler CPU-clock samples |
| All named file-backed faults | 0.270 | 55 pre-handler samples, including 22 from Mesh instructions |
| Anonymous or copy-on-write faults | 0.245 | 50 pre-handler samples |
| `pthread_getattr_np` | 0.182 | 37 pre-handler samples |
| Thread creation | 0.132 | 27 pre-handler samples |

The kernel exec path contributed 0.255 sampled CPU-ms/job. The dynamic loader contributed 0.334 in user space, with 67 of its 68 instruction samples unresolved below the DSO level. This was the leading named startup path in the trace. Moving Mesh sections would require evidence of which file-backed pages the process touched. A Mesh instruction can also fault a page from another image. The 6.91% full-drain shift also disqualifies the profile from pricing a candidate. [Worker pricing](recovered/pricing.json)

Any later worker investigation would need a separate SETUP slot for a lower-overhead test of exec and dynamic-loader costs, with adjacent trace-off controls. The largest independently measured backend scope was Jobs PostgreSQL. Its CPU bounds cover the entire scope and leave claim and settlement CPU unresolved. The decision here ends the worker layout and startup-candidate screen.

## Replay the receipts offline

From this directory, check the hashes and replay both receipts with Python 3. The archived verifier and workload inputs match the baseline source bytes.

```sh
shasum -a 256 -c SHA256SUMS
export PYTHONPATH="$PWD/replay/frozen/tools/durable-jobs-bench"
for capture in initial recovered; do
	python3 -B replay/reduce.py "$capture/evidence.tar.gz" | cmp - "$capture/reduced.json" || exit 1
	python3 -B replay/price-worker.py "$capture/evidence.tar.gz" "$capture/reduced.json" |
		cmp - "$capture/pricing.json" || exit 1
done
```

Each `dependencies.tar` is the exact dispatched input. Each
`evidence.tar.gz` contains the sanitized native receipt, including raw
frames and ledgers. The recovered raw receipt also holds perf data,
decoded call chains, exact wait records, and the per-arm artifact manifests.
