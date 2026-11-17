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
  --bin-dir .tmp/performance/bin --out .tmp/performance/smoke
```

The smoke command runs the workload oracle and checks the actual HTTP
responses. CI runs this command on Ubuntu x86-64. Use smoke results to check
behavior and process cleanup. Shared CI and laptops can produce different
timings from a dedicated host.

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
4. Run five shuffled blocks for each CPU budget. Use 15 seconds of warm-up
   and 30 seconds of measurement as a starting point. Increase the window if
   observations drift or yield too few completions for p99. Run the sustained
   case at the same below-saturation rate for all four stacks. Use it to
   inspect errors and process memory over time.
5. Repeat the same matrix on a fresh host pair. Keep the run directories and
   reports together. Delete the benchmark instances after copying the evidence
   off them. See the [provider pricing](https://www.akamai.com/cloud/pricing/cpu)
   and [network transfer rules](https://techdocs.akamai.com/cloud-computing/docs/network-transfer-usage-and-costs).

Use `python3 tools/performance/run.py run --help` for the rate, duration, and
remote load host flags. Use `python3 tools/performance/run.py report --help`
to render the captured runs. Use a fresh output directory for each attempt.

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
- Before publishing numbers, record the host and CPU model, versions,
  affinity, runtime flags, observed limitations, and raw trial directory.
  Publish the benchmark commit with the results.
