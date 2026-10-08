# Brag Plan: Mesh (v2, two cuts)

v1 (in `v1/`) re-animated the landing page over a stock chord loop and was rejected as basic and generic. v2 is two original films; the user picks one. Style frames for the three candidate directions are in `style-frames.png`.

## What Mesh is
A compiled, statically typed, actor-based language: Elixir-style syntax, Hindley–Milner inference, LLVM native binaries, clustering owned by the runtime.

## Claims on screen, and where they come from
| Claim | Source |
|---|---|
| "Then a hundred thousand" actors | `tests/e2e/actors_100k.mpl`, compiled and run here: spawns and messages 100,000 actors |
| Wrong message won't compile, E0014 | real `meshc build` output for the site's counter sample |
| The runtime reroutes around a lost node | site copy: "Mesh handles placement, routing, and failover" |
| 29,108 req/s, 2.3× Elixir | `benchmarks/RESULTS.md`, isolated peak, GET /text |
| HTTP, WebSockets, Postgres, SQLite, JSON, jobs, tests ship with it | README and stdlib docs |
| `curl -sSf https://meshlang.dev/install.sh \| sh` | README |

## A: Living cluster (22 s, `brag-a-cluster.mp4`)
One continuous 3D world rendered in WebGL: 16k actor particles, messages as light comets, motion blur, depth of field.

| Time | Beat |
|---|---|
| 0.3 | One actor lights up; it doubles generation by generation into a cluster. "One actor. Then a hundred thousand." |
| 3.2 | Messages streak between actors. "Every light is an actor. Every trail, a message." |
| 6.4 | Dive into one actor's mailbox (`Pid<Int>`): `5` and `42` land, `"five"` is rejected with a red shockwave and E0014. |
| 10.4 | Pull back: the cluster ships to two new nodes. "Clustering lives in the runtime." |
| 12.8 | A node flashes red and dies; its traffic swings to the survivors, which grow. "Lose a node. The runtime reroutes." |
| 15.0 | The swarm streams ahead of the camera; 29,108 req/s counts up. "Compiled to native." |
| 18.4 | Every particle lands in the Mesh logo; lockup, tagline, install command. |

Score: sonified. Each spawn generation is a step of a rising pluck arpeggio; message arrivals, quantised to sixteenths, are the melody (pitch from where each landed) and every arrival is a click in the crackle; the rejection crunches into silence; the lost node tape-stops the track; D minor resolves to D major on the logo.

## C: Editorial (20 s, `brag-c-editorial.mp4`)
Brutalist poster sequence, hard cuts on the beat: mint, ink and paper fields, giant condensed Archivo, RGB-split slams.

M / E / S / H → MESH · 01 ACTORS (spawn, send, receive; `send(pid, "five")` WON'T COMPILE) · 02 NODES (node-2 struck out, LOSE A NODE, THE RUNTIME REROUTES, `HTTP.clustered(hello)`) · 03 29,108 req/s, 2.3× ELIXIR, LLVM → NATIVE BINARY · 04 HTTP / WEBSOCKETS / POSTGRES / SQLITE / JSON / JOBS / TESTS, IN THE BOX · logo lockup and install command.

Score: drum-led glitch in F minor at 120 BPM, swung hats, gliding 808 riff, chord stabs on every slam, stutters after the big cuts, bit-crushed drums on the red error card, a tape stop when the node dies.

## Audio
Both scores are synthesised in numpy (`work/synth.py`), mastered to -14 LUFS with a true-peak limiter at -1.5 dBTP (`work/master.py`).

## A, extended (57.6 s, `brag-a-cluster-extended.mp4`), the pick
Same world, cut to "Futuristic Pulse" by Universfield (85.43 BPM, B minor), every beat on the track's downbeats. Holds are longer, the camera moves are slower, and small type is bigger (error card 30 px, tags 34–40 px pills, HUD 20 px).

| Video time | Music | Beat |
|---|---|---|
| 0.8 | intro | one actor lights up |
| 3.7 | intro | cascade: "Then a hundred thousand." |
| 6.5 | intro | messages: "Every light is an actor. Every trail, a message." |
| 14.9 | drop | dive into the mailbox; `5` (17.7) and `42` (19.1) land |
| 20.5 | main | `"five"` rejected, E0014: "Wrong message? It won't compile." |
| 23.3 | main | new: supervision tree; a child crashes (24.7), the supervisor restarts it (26.1). Source: concurrency docs ("a panic ends only the actor it happens in… a supervised actor is restarted") |
| 28.9 | main | pull back: "Clustering lives in the runtime." |
| 34.6 | breakdown | node-3 dies: "Lose a node. The runtime reroutes." |
| 43.0 | return | the flight; 29,108 req/s, 2.3× Elixir |
| 48.6 | outro | the swarm lands in the logo; lockup and install command |
