# Downstream HTTP response persistence fix

This directory vendors crates.io `ureq-proto` **0.5.3**, used by the locked
`ureq` **3.2.0**. The published crate archive SHA-256 is
`d81f9efa9df032be5934a46a068815a10a042b494b6a58cb0a1a97bb5467ed6f`;
its `.cargo_vcs_info.json` identifies upstream commit
`4182eed4c07f8670e1505f7df128f21aaeaf49eb` in
<https://github.com/algesten/ureq-proto>. `UPSTREAM.json` records every original
file hash before this patch. The upstream MIT license text is preserved in
`LICENSE-MIT.txt`; upstream package metadata declares `MIT OR Apache-2.0`.
The cache marker and editor settings are not included.

`src/client/recvresp.rs` previously prohibited reuse only for an exact
`Connection: close` header or close-delimited body. A length-delimited HTTP/1.0
response could therefore return an open but nonpersistent socket to the pool.
A second POST could be sent to that socket before the peer's delayed EOF.

The downstream patch follows [RFC 9112 section 9.3](https://www.rfc-editor.org/rfc/rfc9112.html#section-9.3):
`close` takes precedence, HTTP/1.1 remains persistent by default, and HTTP/1.0
requires an explicit `keep-alive` connection option. Connection options are
matched case-insensitively across comma-separated and repeated headers.
`src/close_reason.rs` names the version-driven closure. An existing closure
reason already prevents pooling, so this new reason is inserted only into an
empty reason list; its four-slot capacity is unchanged.

`src/client/test/state_cleanup.rs` covers response-version/body framing,
bodyless responses, redirects, connection token precedence, and previously
required closure reasons. Mesh runtime and generated opt0/opt2 tests issue
exactly two sequential POSTs through the same client. Their owned peer waits
100 ms before closing the first nonpersistent socket and records connection
IDs and request bodies, refusing accidental reuse or replay. HTTP/1.1 and
explicit HTTP/1.0 keep-alive controls must reuse one connection.

Both the root and independent `fuzz/` workspace patch this exact directory.
The workspace and runtime pin `ureq = "=3.2.0"` so the override cannot silently
be bypassed by a newer incompatible protocol version. The preexisting fuzz
lock used `ureq 3.3.0`/`ureq-proto 0.6.0`; it is aligned to this reviewed stack
(with its corresponding `utf-8` dependency).
No blanket pooling disable, HTTP API change, retry, or application workaround
is introduced. Remove the vendor override only after an upstream successor
passes these same regression tests.
