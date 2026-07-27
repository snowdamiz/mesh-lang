# mesh-solana

Typed Solana primitives, account decoding, and instruction inspection for Mesh.

The package provides canonical `Pubkey`, `Signature`, and `Hash` parsing;
typed JSON-RPC requests/responses; account, slot, and block-height decoding;
program-account filters; WebSocket account/program/slot subscriptions; SPL
token account and mint layouts; and the SPL stake-pool fields used by JitoSOL.
It also ingests bounded Jupiter raw-instruction JSON into typed account metadata
and emits a program, signer, writable-account, and data report for allowlisting.

The account decoders validate the SPL stake-pool and token-program owners.
`jitosol_nav` then validates the deployed Jito stake-pool and mint addresses,
mint supply, nine-decimal configuration, and current epoch before calculating:

```text
floor(total_pool_lamports * 1_000_000_000 / pool_token_supply)
```

The multiplication and division use checked `U128` operations. This package
does not construct messages, sign, simulate, or submit transactions.
