# gitmesh-accounts

Product account, profile, session, and repository ownership infrastructure.

This crate intentionally stays below the web layer. It does not implement HTML
forms, OAuth, cookies, or browser sessions. It provides deterministic validation
and durable local snapshots that `gitmeshd`, `gm`, and the future web gateway can
share.

Implemented:

- username validation and reservation
- account profile records linked to protocol `AccountId`
- opaque session-token issuance with hashed-at-rest tokens
- session revocation and expiry checks
- repository namespace ownership records
- TSV snapshot persistence for local daemon prototypes
