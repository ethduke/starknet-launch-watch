# Starknet Launch Watch

A small, read-only Rust bot that observes **Unruggable token creation/launch events and Ekubo pool initialization**, enriches them with contract getter/ABI observations, and sends console or Telegram alerts. Alerts include explorer links and an AVNU trading-page button; users connect and sign in their own wallets.

This is an early experiment, not an audit service. Coverage is the configured deployments, not every launchpad on Starknet. An existing token can open a new pool. Liquidity is currently the pool's **active liquidity parameter L in raw protocol units**, not USD TVL, reserves, price impact, or executable depth. A zero L can mean liquidity is outside the current price range. Factory events without a pool key report liquidity as unknown. There is no inference that a token is safe because a getter or ABI entrypoint is absent.

## Start

Install Rust (the development build was tested with rustc/cargo 1.98.1), then:

```sh
cargo run --locked -- demo
cp .env.example .env
chmod 600 .env
```

Edit `.env` locally. Do not paste secrets into issue reports, chats, shell arguments, or logs.

| Variable | Required? | Meaning |
| --- | --- | --- |
| `STARKNET_RPC_URL` | For network commands | Starknet mainnet HTTP JSON-RPC endpoint; authenticated providers are supported |
| `TELEGRAM_BOT_TOKEN` | For Telegram | BotFather token; blank disables Telegram when chat ID is also blank |
| `TELEGRAM_CHAT_ID` | For Telegram | Destination numeric chat ID or supported channel username |
| `STARKNET_RPC_WS_URL` | Optional | Provider's WebSocket endpoint supporting event subscriptions |
| `START_BLOCK` | Optional | First block to scan with an empty watch database; default starts after the head observed at startup |
| `POLL_INTERVAL_SECS` | Optional | Recovery polling interval; default 5 seconds |
| `BLOCK_BATCH_SIZE` | Optional | Scan range size, 1–1000 blocks; default 100 |
| `DATABASE_PATH` | Optional | Watch checkpoint/outbox SQLite file; default `data/watch.sqlite` |
| `REGISTRY_PATH` | Optional | Reviewed deployment manifest; default `config/mainnet.json` |

The bot needs permission to message its destination. For a direct conversation, open the bot and send `/start`. For a group, add the bot; for a channel, give it posting permission. Numeric private chat/group identifiers remain local configuration.

```sh
cargo run --locked -- doctor
cargo run --locked -- watch --console --once
cargo run --locked -- watch
```

`doctor` verifies mainnet, reviewed class hashes and event ABIs. It does not send a Telegram message or prove the provider supports WebSocket subscriptions. `watch --console` always forces console delivery. Without Telegram variables, plain `watch` also uses console delivery. Both Telegram variables must be configured together for live Telegram delivery. No blockchain signing key is needed. A paymaster and LLM are not needed for event detection.

## Historical replay

Public mainnet receipt fixtures are committed for an offline demo. To verify the complete RPC pipeline against the same period:

```sh
cargo run --locked -- replay --from 615706 --to 615710
```

Replay defaults to console output and a separate `data/replay.sqlite`. Rerunning an already processed range is deduplicated. Use a different `--database data/another-replay.sqlite` for an independent replay. `replay --telegram` explicitly opts into historical delivery and needs both Telegram variables; `--console` takes precedence.

The registry contains the current reviewed Ekubo class and one reviewed historical class used by the fixtures. Other historical classes stop replay until reviewed. Old and current `PoolInitialized` layouts differ: the old event includes a final `call_points` byte.

## How it works

1. Validate chain ID, deployment class hashes and event layouts against the registry.
2. Query only configured contract/event selectors in bounded accepted-block ranges.
3. Follow every `getEvents` continuation token, then retrieve candidate transaction receipts.
4. Use **receipt event indices** for event identity. This preserves two identical events in a transaction and avoids provider-version differences in event-page indices.
5. Verify receipt canonical block hashes and deployment classes; decode creation, launch and pool events separately.
6. Read metadata, supply, owner/paused getters, checked control entrypoint names, and pool active liquidity at the event's block. Unavailable enrichment becomes unknown.
7. Commit observations, delivery queue entries and the range checkpoint in one SQLite transaction.
8. Deliver queued alerts. Restart resumes from the checkpoint and reconciles canonical hashes.

Optional filtered WebSocket subscriptions wake this loop; HTTP polling remains enabled and does the authoritative recovery scan. Disconnects never advance checkpoints. Provider errors suppress response messages and complete endpoint URLs, because either can contain credentials.

The last 128 range checkpoints are retained. A detected reorg rewinds to a retained matching checkpoint, removes reverted observations, and queues corrections for delivered alerts. If no retained checkpoint matches, the worker stops scanning and requires a reviewed replay into a fresh database. A single worker locks each on-disk database.

Telegram delivery is **at least once**: a timeout or crash after Telegram accepts a message but before local acknowledgement can cause a duplicate. Console and Telegram queues remain separate; switching modes does not send previously delivered console observations to Telegram. Large outages or backfills can delay delivery. The service prints block progress and request counts so RPC usage can be measured.

The service never calls state-changing Starknet RPC methods. The Trade button opens `https://app.avnu.fi/`; it does not preselect a token or promise a route. Copy the canonical token address from the alert. Internal execution and automatic buying are outside this version.

## Security and public repository

- `.env`, databases, logs, signing material, and build artifacts are ignored. Only `.env.example` has placeholder variables.
- RPC and notification clients do not expose credential-bearing endpoints through `Debug`, transport errors, or provider error bodies. Redirects are disabled for HTTP requests.
- RPC methods are explicitly allowlisted as read-only. Mainnet enforcement prevents cross-network mixups.
- Contract metadata is treated as untrusted, length bounded, and stripped of control/direction characters. Telegram uses plain text, with no markup parsing.
- Source class changes pause scanning. Review actual event ABIs and update the registry deliberately, using a new database and explicit replay start to preserve coverage. A class pin is compatibility evidence, not proof of safety.
- Dependency versions are locked. CI uses read-only repository permissions, checks formatting, runs tests and Clippy, and screens tracked files for common secret patterns.
- Third-party AMM implementation code is not vendored. Registry entries are deployment and interface facts; upstream code visibility does not grant a right to copy implementations.
- `.env` and the database are still readable by their local process owner. Run on a dedicated host/user with restricted secret-file permissions and a spending-limited RPC subscription.

See [SECURITY.md](SECURITY.md) and [deployment provenance](docs/SOURCES.md).

## Validation

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
python3 scripts/check_public_files.py
```

Tests cover real historical event layouts, full-width quantities, metadata decoding, event identity, pagination, atomic checkpointing, deduplication, unknown permissions, reorg correction queues, chain/deployment mismatch and suppression of provider secrets.

## Next useful additions

- Match factory launches to pool observations and group related alerts.
- Detect funding added after pool initialization, and update the original observation.
- Add bounded, read-only executable quote/depth checks and trustworthy quote-asset pricing.
- Add launchpad adapters only after confirming deployed addresses, event layouts and actual activity.
- Add wallet-based trading with explicit quote, slippage, expiry and user confirmation.

MIT applies to our code. Third-party protocols retain their own licensing and terms.
