# Project working agreements

- Keep this repository public and publish only this project's reviewed files.
- Never display, commit or transmit `.env`, Telegram tokens, credential-bearing RPC URLs, real private chat IDs, local databases, logs or wallet keys.
- Read-only Starknet RPC is the only backend chain interaction. No transaction signing or submission in the indexer.
- Use bounded event filters, paginated catch-up and persistent checkpoints. Do not advance checkpoints after a partial failed range.
- Treat names, symbols, ABIs and third-party responses as untrusted input. Suppress endpoint and provider error bodies in logs.
- Keep token creation, launch, pool initialization and usable liquidity distinct. Unknown permissions remain unknown; do not add "safe" or "unruggable" verdicts from ABI inspection alone.
- Pin reviewed deployment classes and record live/historical ABI provenance. Review source upgrades before changing adapters or pins.
- Do not copy restrictive-license protocol implementations into this repository.
- Before pushing, run formatting, tests, Clippy and `scripts/check_public_files.py`; inspect the staged file list. Do not print file contents from secret files.

