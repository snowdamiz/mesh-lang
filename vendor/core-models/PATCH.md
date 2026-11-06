# Proof-model dependency metadata

Base: published `core-models 0.0.7`, crate checksum
`ee3344d349528f449816cfa85e558c439bdca5a6d8a738cfb21e69f0b2b1952f`,
published from libcrux commit `c5fb80f37530ee9b2df9501ae5ff8cb4a973a4bd`.
All published Rust and F* source bodies are unchanged. Package provenance and
the supplied Apache-2.0 license are recorded in `UPSTREAM.json`.

Only the normalized `Cargo.toml` requirement `dependencies.hax-lib.version`
changes from `0.3.7` to `0.4.1`. The same requirement is selected by the
[libcrux workspace at the ML-KEM provider's pinned commit](https://github.com/celabshq/libcrux/blob/42d68bd49b244b09bd02626c6ffa95e54dc64f91/Cargo.toml),
where core-models already inherits it. Cargo.lock pins hax-lib and its matching
macros to `0.4.2`. This removes the old proof-macro chain's unmaintained
`proc-macro-error2` dependency without an audit ignore or release-policy change.

Mesh reaches core-models only through libcrux-intrinsics's `cfg(hax)` dependency;
it is absent from the normal runtime graph. Standalone Rust model compilation
and tests check compatibility with the updated macro package. They do not run
hax extraction or the F* proof chain, and no preservation/reverification of the
formal proofs is claimed. No upstream model algorithm or runtime crypto body
is patched. Remove this override when an upstream published package carries
the maintained metadata and its replacement is qualified.
