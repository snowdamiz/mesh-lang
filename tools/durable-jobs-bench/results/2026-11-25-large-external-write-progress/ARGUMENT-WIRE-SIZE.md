# LARGE argument bytes on the wire

Independent Zstandard level-1 frames reduced 33,558,976 bytes of PostgreSQL JSONB text to 16,855,307 bytes across the 64 actual LARGE arguments. Each frame decoded to its original bytes. Binary transport retained the 49.8% reduction. Unwrapped base64 took 22,473,824 bytes. PostgreSQL’s default hexadecimal bytea text representation would take 33,710,742 bytes, slightly more than sending the arguments as text.

This proof uses PostgreSQL 17.11 to render the original arguments. It verifies the native input manifest and each job’s payload digest in all five stock FILE EXTERNAL ledgers. The frozen payloads contain random bytes encoded in hexadecimal. The measurement used a general codec with no dictionary or knowledge of the fixture. Counts cover the argument field, excluding the context envelope, TLS and other framing.

The stock checked-cursor diagnostic supplies a scale comparison. If its entire generous within-grant overlap illustration shrank in proportion to the removed bytes, a binary codec would receive 69.7 and 74.7 ms of credit. The two concurrent prepared arms needed their last effects 84.8 and 76.3 ms earlier for the most permissive historical Oban deadline. That arithmetic already treats all illustrated time as byte-dependent and charges zero for compression and decompression. The actual prepared path has different scheduling and lock timing. Its native codec cost and wall savings are unknown.

Keep transport compression off the native queue on this evidence. The separate first-row streaming experiment acts on when a checked row becomes available. Its timing opportunity cannot be added to these figures as if the two mechanisms removed independent work. [Stock delivery intervals](../2026-11-24-public-large-checked-cursor/RECEIVE-INTERVALS.md) · [Prepared effect deadlines](factorial-deadlines.json)

With PostgreSQL 17.11, Zstandard CLI 1.5.7 and the checkout’s existing `target/` directory, run from this directory:

```sh
python3 -B argument_wire_size.py --pg-bin /opt/homebrew/opt/postgresql@17/bin | cmp - argument-wire-size.json
```
