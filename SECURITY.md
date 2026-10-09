# Security

Please use the repository's private vulnerability reporting feature when available. Do not put bot tokens, RPC URLs, private chat IDs, local databases, or user keys in public issues. If private reporting is unavailable, open an issue asking for a private reporting channel without describing the exploitable details or including secrets.

This service observes chain data and sends alerts. It has no blockchain transaction signer. A token control entrypoint or owner getter is an observation, not a proof that all privilege paths have been enumerated. Empty ABI results and missing getters do not certify absence of permissions. Active liquidity L is not USD liquidity or proof that a trade can execute.

Rotate exposed Telegram and RPC credentials at the provider immediately. Deleting an exposed Git commit is insufficient. Keep credentials outside version control and runtime error reports. Always review staged files before publishing.

Audit and test any future wallet integration separately. Preserve wallet-held keys, network checks, slippage/deadline bounds, user authorization and clear transaction previews.

