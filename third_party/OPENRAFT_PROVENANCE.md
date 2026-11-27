# Vendored OpenRaft 0.9.24 Provenance

This directory contains vendored copies of the upstream `openraft` and
`openraft-macros` 0.9.24 crates with only the `chrono`-removal backport from
`databendlabs/openraft#2387` applied.

## Original registry crates

| Crate | Version | Registry source | Tarball SHA-256 | Upstream VCS commit (`.cargo_vcs_info.json`) | Path in VCS | License |
| --- | --- | --- | --- | --- | --- | --- |
| `openraft` | `0.9.24` | `registry+https://github.com/rust-lang/crates.io-index` | `80d35e2f60cdf9bcfc39a020966091017c6dc2a4b43b355a22ca3e76106f4a0a` | `da1c293692632005f82d180833a9085e3c192669` | `openraft` | `MIT OR Apache-2.0` |
| `openraft-macros` | `0.9.24` | `registry+https://github.com/rust-lang/crates.io-index` | `bbbf0342d747a8da209c8e1d3ca8f788100966669412aaacb449409205931251` | `da1c293692632005f82d180833a9085e3c192669` | `macros` | `MIT OR Apache-2.0` |

### Retained upstream licenses

Both crates declare `license = "MIT OR Apache-2.0"`. The upstream license texts
from `databendlabs/openraft` commit `da1c293692632005f82d180833a9085e3c192669`
are retained verbatim in both `third_party/openraft/` and
`third_party/openraft-macros/`:

- `LICENSE-APACHE` SHA-256: `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2`
- `LICENSE-MIT` SHA-256: `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3`

### Omitted crates.io publish-time artifacts

The `.crate` archives also included `Cargo.lock` (publish-time lockfiles
listing `chrono`) and `Cargo.toml.orig` (upstream workspace-inheriting
manifests). Those two packaging artifacts were omitted from both vendored crate
directories; all crate source files, tests, docs, `README.md`, `Cargo.toml`,
and `.cargo_vcs_info.json` were imported verbatim in commit
`f13a0d068045a51ee398e71fb684ec00da1a6f78`.

## Applied upstream patch provenance

- **Upstream PR**: <https://github.com/databendlabs/openraft/pull/2387>
- **Upstream title**: `fix: deps: remove the chrono dependency`
- **Upstream merge commit**: `bb3aa5313ea7271996bee9470c7c8b8cd9d1482f` (merged 2026-11-18T08:34:48Z)
- **Advisory context**: `RUSTSEC-2026-0543` (`chrono` unmaintained)

## Exact scope of modifications on top of 0.9.24

Only the two `chrono` call sites present in 0.9.24 and the two corresponding
`[dependencies.chrono]` manifest entries were modified (upstream PR #2387 also
updated `openraft/src/metrics/serde_instant.rs` and `display-more`, neither of
which exists in 0.9.24):

1. `third_party/openraft/Cargo.toml`: removed `[dependencies.chrono]`.
2. `third_party/openraft/src/display_ext/display_instant.rs`: replaced
   `chrono::DateTime` formatting in `DisplayInstant` with PR #2387's
   `std::time`-based UTC formatter (`DisplayUtc`, `unix_secs_micros`,
   `civil_from_days`) and added PR #2387's `test_display_utc` and
   `test_civil_from_days` unit tests while retaining 0.9.24's
   `TokioInstant::now()` in `test_display_instant`.
3. `third_party/openraft-macros/Cargo.toml`: removed `[dependencies.chrono]`.
4. `third_party/openraft-macros/src/since.rs`: replaced
   `chrono::NaiveDate::parse_from_str` in `Since::set_date` with PR #2387's
   `is_valid_date` / `parse_digits` calendar validator and added PR #2387's
   `test_is_valid_date` unit test.

No consensus, election, replication, network, storage, or other OpenRaft source
files were modified.
