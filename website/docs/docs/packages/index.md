---
title: Packages and Registry
description: Declare, resolve, install, publish, and use Mesh packages, including the shipped Borsh, Anchor, and Solana packages.
---

# Packages and Registry

Mesh projects can consume source packages from the public registry, a git
repository, or a local path. `mesh.toml` declares intent; `mesh.lock` records
the exact resolved source used by a build.

Two commands intentionally split the work:

- `meshpkg` authenticates with the registry and publishes, searches, or
  installs registry packages.
- `meshc deps` resolves git and path dependency graphs and writes their lock
  entries.

`meshc build` does not fetch missing packages. Resolve dependencies before
building and commit `mesh.lock` with the project.

## Declare dependencies

Registry packages use exact versions:

```toml
[package]
name = "my_app"
version = "0.1.0"

[dependencies]
"your-login/json-tools" = "1.2.0"
local_rules = { path = "../local-rules" }
remote_rules = { git = "https://github.com/example/remote-rules.git", tag = "v1.4.0" }
```

Registry names contain `/`, so quote them as TOML keys. Version ranges are not
supported: `"1.2.0"` is valid, while `"^1.2"` and `">=1"` are not.

Git dependencies accept one of:

```toml
by_commit = { git = "https://github.com/example/lib.git", rev = "full-commit-id" }
by_tag = { git = "https://github.com/example/lib.git", tag = "v1.0.0" }
by_branch = { git = "https://github.com/example/lib.git", branch = "main" }
```

Use an immutable `rev` for release builds. A branch can move between
resolutions even though the resulting lockfile pins the checkout used.

## Install and lock

Install every registry dependency declared in the manifest:

```bash
meshpkg install
```

That command:

1. reads exact registry versions from `mesh.toml`;
2. reuses exact entries already present in `mesh.lock`;
3. downloads and SHA-256 verifies each package;
4. extracts it under the project's package cache; and
5. updates `mesh.lock`.

To inspect the latest release of one package:

```bash
meshpkg install your-login/json-tools
```

A named install downloads the latest registry version and records it in
`mesh.lock`, but deliberately does **not** edit `mesh.toml`. Add the printed
exact dependency declaration yourself before relying on it in a build.

Resolve git and path dependencies separately:

```bash
meshc deps
```

Both commands accept the current project by default. Run them again after
changing dependency declarations.

## Search and browse

Search package names and descriptions:

```bash
meshpkg search json
```

Browse package metadata, version history, checksums, and download counts at
[packages.meshlang.dev](https://packages.meshlang.dev).

Every registry-aware command can target another compatible registry:

```bash
meshpkg search json --registry https://registry.example.com
meshpkg install --registry https://registry.example.com
meshpkg publish --registry https://registry.example.com
```

Add global `--json` for machine-readable output without spinners or color:

```bash
meshpkg --json search json
meshpkg --json install
```

## Publish a package

Open [packages.meshlang.dev/publish](https://packages.meshlang.dev/publish),
sign in with GitHub, and copy the generated publish token:

```bash
meshpkg login --token <your-token>
```

Without `--token`, `meshpkg login` prompts on standard input. Credentials are
stored in `~/.mesh/credentials`.

The package name must be scoped to the authenticated GitHub login:

```toml
[package]
name = "your-login/your-package"
version = "1.0.0"
description = "What the package does"
license = "MIT"
```

Publish from the package root:

```bash
meshpkg publish
```

The client builds a gzip-compressed tar archive, computes its SHA-256, and
uploads it with the manifest metadata. The registry verifies the digest and
ownership scope before making the version visible.

Publishing rules:

- Package versions are immutable. Change the version before republishing.
- The upload limit is 50 MiB.
- Hidden paths and `*.test.mpl` files are excluded.
- Visible `.mpl` modules keep their package-relative paths.
- `mesh.toml` is included.
- Manifest-declared native binding files and static libraries are included and
  rechecked before upload.
- A project `README.md` is not currently included by `meshpkg publish`; do not
  rely on registry README rendering as the package's only documentation.

Keep examples and API guidance in public source documentation as well as in the
package itself.

## Package modules

Package source paths become Mesh module names. For example:

```mesh
from Anchor.Validator import discriminator
from Solana.Read import pubkey
from Solana.Tx import compile_message_v0
```

Only `pub` declarations are importable. A package can use an alternate
application entrypoint through `[package].entrypoint`, but reusable libraries
normally expose modules and do not define the consuming application's startup
path.

## Shipped package: `mesh-borsh`

`mesh-borsh` is a bounded native Borsh reader and writer. It is source-only in
the Mesh repository: the consumer or publisher builds a static library for the
exact host, computes its SHA-256, and declares the resulting ABI 1 archive.
It is not a portable prebuilt archive.

Import its binding module with:

```mesh
from Bindings.Borsh import reader, read_u64, read_bool, finish_reader
from Bindings.Borsh import writer, write_u64, write_bool, finish_writer
```

Reader API groups:

- lifecycle: `reader`, `remaining`, `finish_reader`, `close_reader`;
- signed integers: `read_i8`, `read_i16`, `read_i32`, `read_i64`, `read_i128`;
- unsigned integers: `read_u8`, `read_u16`, `read_u32`, `read_u64`,
  `read_u128`;
- values: `read_bool`, `read_fixed`, `read_len`, `read_vec`, `read_string`,
  `read_option_tag`.

Writer API groups:

- lifecycle: `writer`, `finish_writer`, `close_writer`;
- signed integers: `write_i8`, `write_i16`, `write_i32`, `write_i64`,
  `write_i128`;
- unsigned integers: `write_u8`, `write_u16`, `write_u32`, `write_u64`,
  `write_u128`;
- values: `write_bool`, `write_fixed`, `write_len`, `write_vec`,
  `write_string`, `write_option_tag`.

Readers copy their input and enforce the caller's collection limit. They reject
unexpected EOF, invalid booleans, invalid UTF-8, oversized collections, and
trailing bytes. Writers enforce a caller-selected output limit. Finish a
successful handle with `finish_reader` or `finish_writer`; use the idempotent
close functions when abandoning a handle after an error.

See [Native Packages](/docs/native-packages/) before building or distributing
the archive.

## Shipped package: `mesh-anchor` 0.1

`mesh-anchor` is a pure Mesh validator for Anchor account envelopes:

```mesh
from Anchor.Validator import AccountLayout, account_payload, discriminator, versioned_payload
```

Its public surface is:

- `discriminator(account_name)` — first eight bytes of
  `sha256("account:<AccountName>")`;
- `account_payload(data, actual_owner, expected_owner, account_name)` —
  validates both 32-byte owners and the discriminator, then returns the payload
  without the discriminator;
- `AccountLayout` — expected account name, version offset, version byte, and
  minimum payload length;
- `versioned_payload(data, actual_owner, expected_owner, layout)` — performs
  owner/discriminator validation plus bounded layout and version checks.

The returned versioned payload retains the version byte. Decode its fields with
an IDL-generated mapper, handwritten Mesh, or `mesh-borsh`. Program ID, account
name, version, and field order remain explicit at the call site.

## Shipped package: `mesh-solana` 0.2

`mesh-solana` provides typed, bounded Solana reads, subscriptions, instruction
inspection, message construction, and unsigned simulation.

### `Solana.Read`

Core value types:

- `Pubkey`, `Signature`, and `Hash` with canonical base58 parsing and string
  conversion;
- `Slot`, `BlockHeight`, `EpochInfo`, and `LatestBlockhash`;
- `RpcRequest` and `RpcResponse`;
- `AccountInfo`, `AccountsAtSlot`, `TokenAccount`, `Mint`, and
  `StakePoolState`;
- `ProgramAccountFilter`, `AccountNotification`, and `SlotNotification`.

Value constructors and accessors:

- `pubkey`, `signature`, and `hash_value`;
- `pubkey_string`, `signature_string`, and `hash_string`;
- `pubkey_equal`;
- `slot` and `block_height`.

Request builders and response decoders:

- generic: `rpc_request`, `rpc_request_json`, `rpc_response`, `rpc_send`;
- reads: `get_account_info_request`, `get_multiple_accounts_request`,
  `get_slot_request`, `get_block_height_request`, `get_epoch_info_request`,
  `get_latest_blockhash_request`, `program_accounts_request`;
- filters: `memcmp_filter`, `data_size_filter`, `filters_json`;
- response decoding: `account_info`, `multiple_accounts_from_response`,
  `slot_from_response`, `block_height_from_response`,
  `epoch_info_from_response`, `latest_blockhash_from_response`;
- subscriptions: `account_subscribe_request`, `program_subscribe_request`,
  `slot_subscribe_request`, `send_account_subscription`,
  `send_program_subscription`, `send_slot_subscription`,
  `account_notification`, and `slot_notification`.

SPL and JitoSOL helpers:

- program/address constants: `spl_token_program`, `spl_stake_pool_program`,
  `jitosol_stake_pool`, `jitosol_mint`;
- bounded account decoders: `token_account`, `mint`, `stake_pool`;
- `jitosol_nav`, which validates the deployed pool and mint, owner programs,
  mint supply, nine-decimal configuration, and current epoch before using
  checked `U128` arithmetic.

### `Solana.Tx`

Transaction types:

- `AccountMeta`, `Instruction`, `CompiledInstruction`, and `MessageHeader`;
- `LegacyMessage`, `AddressTableLookup`, and `MessageV0`;
- `JupiterInstructionSet` and `SimulationResult`.

Instruction builders:

- `compute_unit_limit_instruction`;
- `compute_unit_price_instruction`;
- `transfer_checked_instruction`;
- `create_associated_token_idempotent_instruction`.

Message and simulation API:

- `compile_legacy_message` and `serialize_legacy_message`;
- `compile_message_v0` and `serialize_message_v0`, including resolved address
  table lookups;
- `serialize_unsigned_legacy_transaction` and
  `serialize_unsigned_v0_transaction`;
- `simulate_transaction_request` and `simulation_result`.

Jupiter ingestion and reporting:

- `instruction_from_jupiter_json`;
- `jupiter_instruction_set_from_json` and `jupiter_instructions`;
- `instruction_report_json`;
- `legacy_message_report_json` and `message_v0_report_json`;
- `jupiter_instruction_set_report_json`.

The package enforces Solana's 1,232-byte transaction limit and bounds
simulation requests and responses. It constructs zero-signature envelopes for
simulation and never enables signature verification for those unsigned
requests.

`mesh-solana` does **not** hold secrets, sign messages, or submit transactions.
Applications must keep signer custody and submission in a separately reviewed
boundary.

## Reproducible package workflow

For an application using all dependency sources:

```bash
meshc deps
meshpkg install
meshc build .
./output
```

Review changes to both `mesh.toml` and `mesh.lock`. A checksum proves that the
download matches the immutable registry artifact; it does not establish that a
package is safe or appropriate for your application.
