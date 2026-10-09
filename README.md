# Starknet Launch Watch

Starknet Launch Watch is a read-only Rust bot. It reads accepted Starknet blocks. It finds token creation and launch events from the configured Unruggable factory. It also finds new Ekubo pools. It sends alerts to Telegram or the console. Each alert shows token data, contract control functions, and explorer links.

The bot saves its progress and continues after a connection failure. It reports unknown contract controls as unknown. The pool liquidity value is the active liquidity parameter, L. It uses raw protocol units, not US dollars. A new pool can contain an existing token. These data do not prove that a token is safe.

The Trade button opens AVNU. Use your own wallet to approve a trade there. Keep RPC access keys and Telegram access keys in the local `.env` file. Read [Setup](docs/SETUP.md) for installation and operation. Read [Security](SECURITY.md) for security limits.
