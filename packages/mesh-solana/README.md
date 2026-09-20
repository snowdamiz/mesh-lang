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
does not hold secrets, sign messages, or submit transactions.
