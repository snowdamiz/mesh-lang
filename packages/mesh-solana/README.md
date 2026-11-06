# mesh-solana 0.2

Typed Solana primitives, account decoding, instruction inspection, unsigned
message construction, and simulation for Mesh.

The package provides canonical `Pubkey`, `Signature`, and `Hash` parsing;
typed JSON-RPC requests/responses; account, slot, and block-height decoding;
program-account filters; WebSocket account/program/slot subscriptions; SPL
token account and mint layouts; and the SPL stake-pool fields used by JitoSOL.
It also ingests bounded Jupiter raw-instruction JSON—individually or as a build
instruction set—into typed account metadata and emits program, signer,
writable-account, and data reports for allowlisting.

`get_account_info_slice_request` adds a bounded `dataSlice` and optional
`minContextSlot` to a base64 account read. It rejects negative ranges and ranges
beyond 10 MiB without overflowing. `account_from_response` returns `AccountAtSlot`
with the account's bytes, owner and context slot; a missing account is an error.
Callers must still check the expected owner, response ID, minimum context, exact
slice length and their trust policy. Morse's native anchor reader dogfoods this
with finalized reads, bounded HTTP responses and redirects disabled.

`get_multiple_accounts_context_request` reads 1–100 ordered accounts from one
bank with an optional `minContextSlot`. The existing
`get_multiple_accounts_request` uses the same bounds without a minimum.
`optional_accounts_from_response` preserves each missing account as `None`
in `OptionalAccountsAtSlot`; malformed accounts, contexts and RPC errors still
fail. The original `multiple_accounts_from_response` requires all accounts to
exist. Callers must check the count, response ID, minimum context and owners.
Unsigned account metadata (lamports, rent epoch and context slots) must be JSON
integers; quoted numbers and fractions are rejected without rounding.
Morse's custody reader uses whole-batch agreement between two finalized RPC
providers so a deposit or withdrawal cannot produce a mixed balance snapshot.

`get_multiple_accounts_slice_request` applies one bounded `dataSlice` to the
whole batch, as supported by [getMultipleAccounts](https://solana.com/docs/rpc/http/getmultipleaccounts).
`optional_accounts_slice_from_response` returns `OptionalAccountSlicesAtSlot`;
each present `AccountSlice` contains `info: AccountInfo` and `space: U64`, the
full allocation rather than the returned byte count. The single-account
`account_slice_from_response` returns `AccountSliceAtSlot` and also preserves
absence. Both require numeric `space` at least as large as the decoded bytes,
strict JSON objects for account/context fields, and an unsigned context slot.
Callers still verify the expected full allocation, returned slice length,
owners, count, request ID, minimum context and provider trust. Morse's attendance
reader uses this to inspect a large posting clock and its weekly counters from
one bank without fetching the clock's entire history.

`program_accounts_context_request` adds `withContext: true` and optional
`minContextSlot` to [getProgramAccounts](https://solana.com/docs/rpc/http/getprogramaccounts).
`program_accounts_from_response(response, maximum_accounts)` returns a
`ProgramAccountsAtSlot` containing keyed accounts and the unsigned context.
The caller chooses a bound from 0–1,000; oversized responses, duplicate pubkeys,
null accounts and malformed values fail instead of yielding a partial set.
Ordering is preserved but not guaranteed by RPC. Callers must bound HTTP bytes
and validate IDs, minimum slots, owners, filters and trust independently.
Morse uses this for whole-set agreement on active and unbonding witness records,
including rotated keys whose bonds are still collectible.

`get_block_time_request(id, slot)` and `block_time_from_response(response)`
implement [the getBlockTime RPC](https://solana.com/docs/rpc/http/getblocktime).
Time is signed Unix seconds; `null` is `None`, and RPC/malformed-value errors
remain errors. Callers establish finality and provider trust separately. The
response decoder preserves JSON value types, so a quoted number cannot become
a timestamp, and rejects responses with both a result and a non-null error.
Morse's anchor freshness check dogfoods this with a two-provider time quorum.

`clock_sysvar_address()` identifies the Clock account. `clock_sysvar(account)`
decodes its exact 40-byte layout into `SolanaClock`: unsigned `slot`, `epoch`
and `leader_schedule_epoch`, and signed Unix-second `epoch_start_timestamp`
and `unix_timestamp`. It rejects a foreign owner, executable or unfunded
account and any truncated or extended layout. Callers authenticate the requested
address, RPC response, context and provider trust; the decoder cannot establish
those from account bytes. Morse's relay admission reads finalized Clock accounts
from independent providers and checks their embedded slots against their RPC
contexts before applying the judge's proof-age rules. This uses the same Clock
as the on-chain judge, rather than an estimated block time or local wall clock.

`Solana.Address` provides `create_program_address(program, seeds)` and
`find_program_address(program, seeds)`. The latter returns a
`ProgramDerivedAddress { address, bump }` with the canonical highest bump,
matching the pinned `solana-address 2.7.0` search from 255 through 1. Creation
allows 16 seeds; finding allows 15 plus the bump; each seed is at most 32 bytes.
Programs must be 32 bytes. Seeds are concatenated exactly as Solana specifies,
without length prefixes, so applications must use fixed-width fields or explicit
framing to prevent seed-boundary collisions. Empty seeds are allowed.

`Solana.LookupTable` prepares create, extend, deactivate and close instructions
and decodes initialized lookup-table accounts. `create_lookup_table_instruction`
returns the derived address and instruction for an authority, payer and unsigned
recent slot. Extensions accept 1–256 addresses and an optional rent payer;
callers must split them to fit their actual transaction packet and remaining
table capacity. Creation matches the official SDK's nonsigning authority meta;
subsequent modifications require the authority's signature. Closure is subject
to the on-chain deactivation cooldown, not a local elapsed-time estimate.

`lookup_table_account` validates owner, funding, exact metadata/padding and
0–256 address entries. It preserves full-width deactivation and extension slots,
the last-extension start index and optional authority. The caller binds the
requested address, authority, address order and trusted RPC context. New entries
cannot be used in the extension slot; deactivating table usability depends on
SlotHashes. A frozen table has no authority. The optimized compiler proof
compares instructions, account roles, PDA derivations and mutable/frozen account
decoding against pinned `solana-address-lookup-table-interface 3.1.0` across 64
cases. Morse uses these APIs for relay preparation and finalized table recovery,
then executes the generated lifecycle instructions in its SBF proof.

`Solana.SlotHashes` exposes `slot_hashes_sysvar_address()`,
`slot_hashes_sysvar(account)` and `slot_hashes_position(hashes, slot)`. It decodes
up to 512 distinct descending unsigned slots and their 32-byte hashes. Both SDK
serialization (`8 + 40 * count` bytes) and the fixed 20,488-byte sysvar allocation
are supported. As in the SDK, unused allocation bytes are not history. Foreign,
executable or unfunded accounts, malformed lengths, duplicate/out-of-order slots
and excessive counts are rejected. The caller authenticates the requested
sysvar address, provider and bank context.

`lookup_table_status(table, current_slot, hashes)` returns `Activated`,
`Deactivating(remaining_blocks)` or `Deactivated`. It matches the pinned SDK's
cooldown rule, including 513 remaining blocks in the deactivation slot even if
that slot is not yet in history. Later, a retained slot returns `512 - position`;
only its absence permits `Deactivated`. Remaining blocks are not a wall-clock
ETA. Future table or history coordinates are errors. The optimized compiler
proof compares compact/padded sysvars and cooldown decisions to pinned
`solana-slot-hashes 3.2.0` and the lookup-table SDK, including skipped slots,
empty/full history and slots above JavaScript's exact-number range. Morse uses
these APIs in its finalized lookup-closure decision quorum.

Address derivation runs in Mesh with public SHA-256 and
`Crypto.ed25519_is_on_curve`. It holds no keys and does not authorize an account
read or transaction. The native compiler proof compares 256 derivations and
512 point encodings with the pinned official SDK. Morse derives its log, ring
and historical witness-set addresses through this package.

Version 0.2 adds legacy and v0 message compilation and serialization,
address-table lookups, recent-blockhash reads, compute-budget and SPL token
instructions, Jupiter instruction-set ordering, zero-signature transaction
envelopes, bounded `simulateTransaction` requests and responses, and reports
that expose message programs/accounts without exposing transaction bytes. The
package validates Solana's 1,232-byte transaction limit and never enables
signature verification for unsigned simulation requests.

The account decoders validate the SPL stake-pool and token-program owners.
`jitosol_nav` then validates the deployed Jito stake-pool and mint addresses,
mint supply, nine-decimal configuration, and current epoch before calculating:

```text
floor(total_pool_lamports * 1_000_000_000 / pool_token_supply)
```

The multiplication and division use checked `U128` operations. This package
does not hold secrets or sign messages. Bounded submission request/result
primitives are documented in [TRANSACTION_READ.md](TRANSACTION_READ.md); the
caller owns endpoint selection, durable intent and authorization.

The same document covers `Solana.TransactionWrite` for authenticated signed
legacy packet assembly and the separate finalized-outcome reader for operational
failure reconciliation. Ordinary finalized payment reads still reject failed
execution.

`Solana.Nonce` adds strict current nonce-account decoding, the canonical System
nonce-advance instruction and unsigned legacy messages with that instruction
first. See [NONCE.md](NONCE.md) for the supported format, authentication boundary,
execution-failure semantics and future-deprecation limitation.
