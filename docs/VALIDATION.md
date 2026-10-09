# Initial validation

2026-10-09, local Rust 1.98.1 development build:

- `cargo fmt --check`: passed.
- `cargo clippy --offline --all-targets -- -D warnings`: passed.
- `cargo test --offline`: 18 tests passed, including public historical and recent receipt decoding, mock-RPC pagination/failure handling, durable outbox/deduplication and correction queues.
- `scripts/check_public_files.py`: tracked-file common-secret-pattern screening passed. This is a limited screen, not a guarantee of absence of secrets.
- `doctor`: mainnet chain ID, current deployment class hashes and event ABIs passed against a public RPC. Reported head: 16136924; provider reported spec `0.10.3-rc.0`.
- Real network replay of block 16119134: one Ekubo pool observation decoded, enriched and printed. Token metadata, owner/paused getters, exposed control entrypoints and raw active liquidity were retrieved. The run used 39 read-only RPC requests including preflight and canonical-state checks.
- Local `.env` is ignored and has mode 0600. Runtime SQLite data is ignored.

Telegram delivery and the selected user's authenticated RPC/WebSocket provider have not been validated yet. No onchain transaction was signed or sent. Liquidity is raw active L; USD depth, trade quotes, liquidity locks, factory/pool alert grouping and additional launchpad coverage remain subsequent work.

