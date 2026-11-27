# Vendored ureq diagnostics

Protected Mesh HTTP exchanges explicitly opt out of payload diagnostics.
The ordinary HTTP path uses the same client with diagnostics enabled.
The scope lives on the blocking worker through response body consumption.
It omits raw HTTP wire dumps, request and response formatting, and
peer-provided content-encoding or cookie text. Transport and protocol-state
diagnostics remain available.

| Crate | Registry version | Registry tarball SHA-256 | Upstream VCS revision |
| --- | --- | --- | --- |
| ureq | 3.5.2 | `3ac5cecf619ca81f66a96482db09687c9f7465af65f4300a8c30cca05bd5a59d` | `51bb001154f44cfab69653a74f7d45fbe1995b22` |
| ureq-proto | 0.7.1 | `622dce33c252115466d83f63e9744a18f12b4406aae51a01c15ad8fcf1185eb3` | `acb5cc3ee1261709316acd6538be08e82f84f3c6` |

Both crates are MIT OR Apache-2.0. The registry license and VCS metadata
files are retained. The vendor copy omits publish-time manifests and
lockfiles, third-party CI/editor metadata, and the upstream ureq test server
certificate and private key. Mesh builds omit ureq's internal `_test`
feature. Run ureq's own TLS fixture tests from upstream source.

The source changes are the closure scope and wire logging check in
`ureq-proto`, and the corresponding checks in ureq's request, response,
body decoder and optional cookie diagnostics. ureq's manifest uses the
adjacent `ureq-proto` directory. Mesh selects ureq by path in both lockfiles.

ureq 3.5.2 includes fixes for timeout budget propagation, bounded TLS
handshakes, and decoded body size limits. Mesh measures TLS time at the
connector and gives the decoded body reader the exact configured limit.
The release evidence audits the original registry identities because
Cargo Audit excludes local dependencies.
