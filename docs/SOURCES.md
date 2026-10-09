# Deployment and event provenance

Verified using read-only mainnet JSON-RPC on 2026-10-09. Head observed during initial discovery: 16136379. This is an observation timestamp, not an assurance about future code.

## Unruggable

- Address: `0x1a46467a9246f45c8c340f1f155266a26a71c07bd55d36e8d1c7d0d438a2dbc`
- Class: `0xd3fbb961f7b54a6b0d9233ab181722d9daa28d6f4a6d86e871d357a7c31200`
- Deployment constant and indexer starting block: [contracts.ts](https://github.com/keep-starknet-strange/unruggable.meme/blob/16487da1dec760dba5d91a29e8d03ae8162ca007/packages/core/src/constants/contracts.ts), [indexer constants](https://github.com/keep-starknet-strange/unruggable.meme/blob/16487da1dec760dba5d91a29e8d03ae8162ca007/packages/indexers/src/constants.ts).
- Events verified from the deployed ABI: `MemecoinCreated` has owner, felt name/symbol, u256 initial supply and token address; `MemecoinLaunched` has token address, quote address and exchange name.
- The registry address is only one configured factory. This does not establish that it covers current Sniq, Launchy or other factories.

## Ekubo

- Core: `0x5dd3d2f4429af886cd1a3b08289dbcea99a294197e9eb43b0e0325b4b`
- Current reviewed class: `0x423df19e032f2d9bf9bb5bc1ea96db2b06d4c752c8c130cba7d577eed1de20a`
- Historical class at block 615710: `0x55aba2d9ef09a504dabbec06a2101cc18eb5b0960517524047c06eae842fa25`
- [Deployment documentation](https://docs.ekubo.org/reference/contracts/starknet/) explicitly identifies Core as upgradeable.
- Current deployed `PoolInitialized` contains PoolKey (token0, token1, fee, tick spacing, extension), signed initial tick and u256 square-root ratio. The historical class adds a final u8 `call_points`. Struct member layouts were checked against `getClassAt` on those blocks.
- `get_pool_liquidity` reads the pool's active liquidity parameter. This is not a token reserve or USD TVL. Pool initialization itself requires no funding.
- Protocol implementation source inspected for interface context: commit `66dc73abadf6cb0bc2f4a728c004ff7558ab6fcb`. No implementation source has been copied or relicensed into this project. [Upstream license](https://github.com/EkuboProtocol/starknet-contracts/blob/main/LICENSE).

## Public receipt fixtures

`tests/fixtures/mainnet-receipts.json` contains accepted transaction receipts from blocks 615706, 615710, 615802 and 615807, fetched through read-only RPC. Four transactions produce two creations, two launches and two pool initializations. Raw source event indices are derived from each receipt's complete event list.

A separate recent receipt at block 16119134 verifies the current nine-value pool event layout (`modern-mainnet-receipt.json`).

These fixtures verify historical decoding. They do not prove present-day activity or current market demand. Recent factory discovery over a 20,000-block window returned no events in the initial probe; this is only that window's result.

## RPC and trading interfaces

- [Starknet WebSocket RPC specification](https://github.com/starkware-libs/starknet-specs/blob/master/api/starknet_ws_api.json).
- [Starknet read RPC specification](https://github.com/starkware-libs/starknet-specs/blob/master/api/starknet_api_openrpc.json).
- [AVNU application](https://app.avnu.fi/): the bot links to this fixed HTTPS destination; no route availability or token-specific deep-link behavior is assumed.
