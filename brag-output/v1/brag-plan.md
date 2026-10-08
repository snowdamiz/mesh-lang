# Brag Plan: Mesh

## What is it
A compiled, statically typed, actor-based programming language: Elixir-style syntax and actors, Hindley–Milner inference, LLVM native binaries, and clustering owned by the runtime.

## Who it's for
Backend engineers who write services: the kind that want Erlang/Elixir's actor model and failover story without giving up static types or native speed.

## What sets it apart
- Actors have **typed mailboxes**: `send(pid, "five")` to a `Pid<Int>` is a compile error (real `meshc` output, E0014).
- **Clustering lives in the runtime.** `HTTP.clustered(hello)` marks work that may run anywhere; placement, routing and failover are the runtime's job.
- **Native speed, measured**: 29.1k req/s on GET /text, next to Go's 30.3k and 2.3× Elixir's 12.4k (benchmarks/RESULTS.md, one run).
- **Server primitives ship with it**: HTTP, WebSockets, Postgres, SQLite, JSON, jobs, a test runner.

## Angle
The site's own promise, shown literally: **Write a server. Ship a fleet.** One node multiplies into a fleet, the fleet folds into the Mesh logo (the logo *is* a hub and spokes), then three proofs: the compiler catches the wrong message, the cluster shrugs off a dead node, the benchmark holds up.

## Hook (0–3.5s)
"Write a server." rises in beside a single glowing node. On the drop, "Ship a fleet." lands and the node fans out into a 12-node mesh with packets moving.

## Tone
- Preset: polished, with cinematic weight on the hook and outro
- Direction: quiet, confident systems-language film in the site's dark theme
- Pacing: 7 scenes, fast entrances, real holds; cuts stagger (old out, new in) over a persistent hairline frame

## Format: landscape 1920×1080, 30fps · Duration: 24.0s (12 bars at 120 BPM)

## Visual identity (from website/docs/.vitepress/theme)
- Background `oklch(0.115 0 0)`, panels `oklch(0.14 0 0)`, heads `oklch(0.165 0 0)`
- Accent (brand) `oklch(0.8 0.15 163)`, hi `oklch(0.87 0.16 160)`; ok/warn/err from the landing's dark palette
- Display: Archivo 600, stretch 114%, tracking -0.045em · Body: Inter · Code: JetBrains Mono
- Code highlighted by shiki with the repo's own `mesh.tmLanguage.json` + `mesh-dark` theme
- Hairline frame with crosses, 24px dot grid, logo = triangle + hub

## Storyboard

| # | Time | Scene | On screen | Sound |
|---|---|---|---|---|
| 1 | 0.0–3.5 | Hook | "Write a server." + one node; 2.0 "Ship a fleet." + node fans out to a 12-node mesh, packets flow | Pad + riser; drop at 2.0 with impact |
| 2 | 3.5–7.0 | Reveal | Fleet nodes converge into the logo; 4.0 "Mesh" slams; README line "An actor-based language for native services and distributed systems."; chips Native · Typed · Concurrent · Distributed | Swell → bell chord hit at 4.0 |
| 3 | 7.0–11.0 | Typed mailboxes | Site's actor sample; `send(pid, "five")` types in; 9.0 real E0014 error from meshc | Key ticks; low dissonant thud at 9.0 |
| 4 | 11.0–15.0 | Failover | Hero's `HTTP.clustered(hello)` line; site's failover sim: cursor clicks "Stop a node", node-2 goes offline, traffic reroutes, log streams, node rejoins. Footnote "Illustrative." kept | Click, glitch drop + filter dip, chime on "traffic rebalanced" |
| 5 | 15.0–18.0 | Native speed | "Native speed, measured." Rust/Go/Mesh/Elixir bars grow; 16.0 "2.3× Elixir" badge | In-key plucks per row; ping on badge |
| 6 | 18.0–20.0 | Stdlib | "Server primitives, included." tabs cycle HTTP client → Testing with real sample code | Soft ticks |
| 7 | 20.0–24.0 | Outro | Logo + Mesh, "Write a server. Ship a fleet.", install command types, meshlang.dev | Full hit at 20.0, drums drop at 22.0, pad rings out |

Every line a viewer must read is settled for at least ~0.3s/word; the log, code and table numbers are texture.

## Audio
Original score composed to picture (numpy synth, A minor, 120 BPM, Am–F–C–G): sub bass, restrained four-on-the-floor, 16th arp with dotted-8th delay, detuned pad, one shared reverb. All SFX are pitched to A minor and sent to the same reverb; they sit under the music. Loudness normalised to -14 LUFS.

## Share copy (draft)
Mesh: write a server, ship a fleet. Typed actors, LLVM-native binaries, and clustering that lives in the runtime, not your handlers.
