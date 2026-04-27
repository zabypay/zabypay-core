# Contributing to ZabyPay

Thanks for your interest in improving ZabyPay! This document covers everything
you need to know to get a working dev environment and submit a clean pull
request.

## Ground rules

- **Be conservative with crypto code.** Anything that touches keys, signing,
  fees, balances, or webhook signatures requires extra review and tests.
- **Never commit secrets.** Use placeholders in `.env.example`.
- **One logical change per PR.** Refactors and feature work in separate PRs.

## Local setup

```bash
git clone https://github.com/Parikalp-Bhardwaj/zabypay
cd zabypay
cp .env.example .env
# fill in secrets — see README.md
docker compose up -d postgres redis
cargo run -p migration -- up
cargo run
```

## Before you push

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo build --release
```

CI runs the same checks; PRs that fail CI will not be reviewed until green.

## Commit style

- Use imperative present tense (`add ...`, `fix ...`, not `added ...`).
- Reference issues in the body (`Fixes #123`).
- Keep the subject line ≤ 72 chars.

## Adding a new chain

1. Add an entity / migration if the chain needs new fields.
2. Add a wallet generator under `src/shared/service/<chain>_wallet.rs`.
3. Add a monitor under `src/shared/service/<chain>_address_monitor.rs` (or
   reuse `universal_address_monitor` if it can be parameterised).
4. Wire it into `payment_monitor` and `withdrawal_processor`.
5. Update `.env.example` with the new RPC variable and document it in the
   README.

## Reporting security issues

Please **do not** open a public issue for security vulnerabilities. Email
[parikalp.123@gmail.com](mailto:parikalp.123@gmail.com) privately first;
we will coordinate a fix and disclosure.
