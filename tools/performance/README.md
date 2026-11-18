# Compare Mesh, Elixir, Go, and Rust

Run this suite to compare the four implementations of a fixed workload. The
[workload contract](contract.md) defines which work each program completes.
Use the correctness run before collecting performance data.

## Build and check the suite

Install the toolchain versions in [versions.env](versions.env), Python 3.11
or newer, and a C toolchain. Set `LLVM_SYS_211_PREFIX` to your LLVM install
directory. For Linux x86-64, use LLVM's official archive and the same linker
settings as the [smoke workflow](../../.github/workflows/performance-smoke.yml).
Install Hex and Rebar with `mix local.hex --force` and
`mix local.rebar --force`. Install the pinned k6 release for HTTP load.

From the repository root, run:

```sh
bash tools/performance/build.sh
python3 tools/performance/run.py smoke \
  --bin-dir .tmp/performance/bin --out .tmp/performance/smoke \
  --require-k6 --k6-version 1.6.1
```

The smoke command runs the workload oracle and checks the actual HTTP
responses. CI runs this command on Ubuntu x86-64. Use smoke results to check
behavior and process cleanup. Shared CI and laptops can produce different
timings from a dedicated host.

## Iterate on a runtime change

Save the baseline Mesh executable before rebuilding. Build the candidate
with the same compiler flags and benchmark source, and use one Elixir release
as the reference for both builds.

```sh
python3 tools/performance/quick.py \
  --baseline .tmp/performance/baseline/mesh \
  --candidate .tmp/performance/candidate/mesh \
  --elixir .tmp/performance/bin/elixir \
  --out .tmp/performance/local-ab
```

The quick run shuffles the two Mesh builds and the reference in each block.
Use `--rust .tmp/performance/bin/rust` in place of `--elixir` for a Tokio
reference. The runner checks actor replies with one and four workers and uses
k6 for a few seconds of connection-per-request HTTP load with four workers.
Use `--workload actor`
to skip HTTP and k6, or `--workload http` to measure the accept path alone.
Compare medians and ranges across the blocks. This is a localhost diagnostic.

To compare actor placement, pass the same current Mesh executable to
`--baseline` and `--candidate`. Add `--workload actor --grouped-candidate`.
The candidate puts each caller and counter in its own group. Both sides check
the same replies and report the placement used. Each block reports round trips/s,
sampled median and p99 latency, and the percentage of samples at or below 1 µs.
That percentage shows whether a change in the median reflects a small shift in the share
of messages getting an immediate reply.
On Linux the runner pins subjects when `taskset` and four CPUs are available.

To measure the compiler's grouped receive optimization in isolation, build
the same program twice against one release runtime. Use the Rust executable
from `build.sh`. Both Mesh executables use the same placement in this run.

```sh
export MESH_RT_LIB_PATH="$PWD/target/release/libmesh_rt.a"
MESH_GROUPED_STEP=0 target/release/meshc build tools/performance/mesh --opt-level 2 --output .tmp/performance/bin/mesh-coroutine
MESH_GROUPED_STEP=1 target/release/meshc build tools/performance/mesh --opt-level 2 --output .tmp/performance/bin/mesh-step
python3 tools/performance/quick.py --workload actor \
  --baseline .tmp/performance/bin/mesh-coroutine --grouped-baseline \
  --candidate .tmp/performance/bin/mesh-step --grouped-candidate \
  --rust .tmp/performance/bin/rust --pairs 64 --actor-iters 160000 \
  --warmup-ops 2048 --blocks 11 --out .tmp/performance/actor-step-ab
```

## Run on two dedicated hosts

1. Create two Akamai Linode G8 Dedicated 8x4 instances in one North American
   data center. Use the same Ubuntu 24.04 x86-64 image for both. Put the
   instances in a private VPC and allow the test port only from the load host.
   Use the server host for all four subjects, one at a time.
2. Build on the server host. Install the pinned k6 release on the load host.
   Check SSH connectivity and the private address before collecting results.
3. Pilot the rate grid. Include a sparse rate and increase toward saturation.
   Confirm the load generator can deliver the offered rate without running
   out of CPU, virtual users, or sockets. Set the same rates for all subjects.
   Use `--blocks 1 --sustained-seconds 30` for a short pilot.
   Raise `--k6-vus` if the client runs out of virtual users before the
   load point you want to study. The report counts unsent arrivals as errors.
4. Run five shuffled blocks for each CPU budget. Use 15 seconds of warm-up
   and 30 seconds of measurement as a starting point. Increase the window if
   observations drift or yield too few completions for p99. Run the sustained
   case at the same below-saturation rate for all four stacks. Use it to
   inspect errors and process memory over time.
5. Repeat the same matrix on a fresh host pair. Keep the run directories and
   reports together. Delete the benchmark instances after copying the evidence
   off them. See the [provider pricing](https://www.akamai.com/cloud/pricing/cpu)
   and [network transfer rules](https://techdocs.akamai.com/cloud-computing/docs/network-transfer-usage-and-costs).

Run the frozen comparison on the server host. Replace the example addresses
and rates with the values from your pilot.

```sh
python3 tools/performance/run.py run \
  --bin-dir .tmp/performance/bin --out .tmp/performance/host-a \
  --load-host bench@10.0.1.20 --server-address 10.0.1.10 --port 18080 \
  --http-rates 10 100 250 --sustained-rate 100 --k6-version 1.6.1
```

After you repeat the comparison, bring both run directories into one checkout.
Use a fresh output directory for each attempt.

Pass `--languages mesh` to `smoke` and `run` to measure Mesh alone. The
runner uses the same workload settings and requires only `mesh` in `--bin-dir`.

```sh
python3 tools/performance/run.py report \
  .tmp/performance/host-a .tmp/performance/host-b
```

## Interpret results

- Compare successful work at the same offered load and CPU budget. Include
  missed arrivals, failed requests, and incorrect outputs in the error rate.
- The HTTP comparison uses one TCP connection for each request. Mesh's
  server closes the connection after its response. Request latency includes
  the cost of setting up that connection. HTTP results apply to the named
  stacks and this wire behavior.
- A supervisor trial measures worker replacement and sibling state. A
  two-host HTTP trial measures remote request/reply over that wire contract.
  Native cluster transports, replicated state, and request replay need
  their own comparison with equivalent guarantees.
- Compare medians and run-to-run spread within each host. If a reported gap
  fits inside the spread or changes direction on the fresh host, treat it as
  inconclusive. Profile a separate run before attributing a gap to a runtime
  or library.
- Confirm short actor and CPU cases ran long enough for memory samples. The
  runner polls the process every 250 ms. An omitted memory value means the
  process exited before a memory sample was available.
- Startup uses a request probe. Compare the bound and probe window together.
  The window reflects the time between the last failed probe and the first
  correct response.
- Before publishing numbers, record the host and CPU model, versions,
  affinity, runtime flags, observed limitations, and raw trial directory.
  Publish the benchmark commit with the results.
