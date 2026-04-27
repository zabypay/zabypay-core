# ZabyPay

[![CI](https://github.com/Parikalp-Bhardwaj/zabypay/actions/workflows/ci.yml/badge.svg)](https://github.com/Parikalp-Bhardwaj/zabypay/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

**ZabyPay** is an open-source, self-hostable crypto payment gateway written in
Rust. It lets merchants accept payments in **ETH, BNB and SOL native
currencies**, generates per-payment receiving addresses, monitors the relevant
chains for incoming transactions, and notifies the merchant via webhooks once
a payment confirms.

> Status: pre-1.0. APIs and database schema may still change.

---

## Features

- **Multi-chain support** — native ETH, native BNB (Smart Chain) and native
  SOL. (Bitcoin and USDT/ERC-20/BEP-20/SPL code paths are present in the
  source tree but disabled at the API boundary in this build.)
- **HD wallet generation** — per-merchant BIP-39 seeds; per-payment derived
  receiving addresses; encrypted at rest with AES-GCM.
- **On-chain monitoring** — background workers poll RPC endpoints, detect
  deposits, and update payment state (pending → confirmed).
- **Webhooks** — signed HTTP callbacks to the merchant on payment events with
  automatic retry.
- **Withdrawals** — single-wallet and multi-wallet withdrawal flows with fee
  estimation per network.
- **API key auth** for merchant API; **JWT** for the merchant dashboard.
- **Rate limiting** via Redis, structured logging, health endpoints.
- **Docker-first** — `docker compose up` brings up Postgres, Redis and the
  API.

---

## Architecture

```
                     ┌──────────────────────┐
   merchant API ───► │  api-crypto (8080)   │
   (REST + JWT)      │  Actix-Web           │
                     └──────────┬───────────┘
                                │
        ┌───────────────────────┼─────────────────────────┐
        │                       │                         │
        ▼                       ▼                         ▼
  ┌────────────┐         ┌─────────────┐         ┌─────────────────┐
  │ PostgreSQL │         │   Redis     │         │  Chain RPCs     │
  │  Sea-ORM   │         │  (cache /   │         │  ETH / BSC /    │
  │            │         │   limits)   │         │  SOL            │
  └────────────┘         └─────────────┘         └─────────────────┘
```

Module layout (`src/`):

| Module      | Purpose                                                        |
|-------------|----------------------------------------------------------------|
| `client/`   | End-user auth (register / login), wallet generation            |
| `merchant/` | Merchant lifecycle, API keys, payments, webhooks, withdrawals  |
| `testnet/`  | Testnet-only payment endpoints                                 |
| `shared/`   | DB entities, services, utilities, background workers           |
| `migration/`| Sea-ORM migrations (separate workspace member)                 |

Background services started at boot:

- `payment_monitor` – polls chains for incoming transactions
- `payment_scheduler` – periodic payment-state reconciliation
- `webhook_service` – webhook delivery with retry/back-off

---

## Quick start

### 1. Prerequisites

- Rust 1.75+ (`rustup install stable`)
- Docker + Docker Compose, **or** local PostgreSQL 14+ and Redis 7+

### 2. Configure

```bash
cp .env.example .env
# generate strong secrets
echo "JWT_SECRET=$(openssl rand -hex 32)"               >> .env
echo "ENCRYPTION_KEY=$(openssl rand -base64 32)"        >> .env
echo "CRYPTO_ENCRYPTION_KEY=$(openssl rand -hex 32)"    >> .env
```

Edit `.env` to point at your database, Redis, SMTP and chain RPCs.

### 3. Run with Docker

```bash
docker compose up --build
```

This starts Postgres, Redis, and the `zabypay` service.
The public API is on `http://localhost:8080`.

### 4. Run locally (without Docker)

```bash
# 1. start Postgres + Redis (e.g. via docker compose up postgres redis)
# 2. apply migrations
cargo run -p migration -- up
# 3. run the API
cargo run --release
```

Health check: `curl http://localhost:8080/health`.

---

## Environment variables

See [`.env.example`](.env.example) for the full list. Highlights:

| Variable                | Required | Notes                                    |
|-------------------------|----------|------------------------------------------|
| `DATABASE_URL`          | yes      | Postgres connection string               |
| `JWT_SECRET`            | yes      | 32-byte hex                              |
| `ENCRYPTION_KEY`        | yes      | 32-byte base64 — used for app secrets    |
| `CRYPTO_ENCRYPTION_KEY` | yes      | 32-byte hex — used for wallet seeds      |
| `REDIS_URL`             | yes      | Used for rate limiting and caching       |
| `ETHEREUM_RPC_URL`      | optional | Required if accepting ETH                |
| `BNB_RPC_URL`           | optional | Required if accepting BNB                |
| `SOLANA_RPC_URL`        | optional | Required if accepting SOL                |
| `SMTP_*`                | optional | Required for verification emails         |
| `CORS_ALLOWED_ORIGINS`  | yes      | Comma-separated list                     |

**Never commit a real `.env`.** It is in `.gitignore`.

---

## API flow (high level)

1. **Merchant signs up** → admin verifies → merchant creates an API key.
2. **Create payment request**
   `POST /api/v1/payments/payment-requests` with API key →
   server returns a payment id, a derived receiving address, and a
   `payment_uri` (BIP-21 / EIP-681 / Solana Pay).
3. **Customer pays** by scanning the QR or sending from their wallet.
4. The on-chain monitor picks up the transaction and marks the payment
   `confirmed` once the configured confirmation depth is reached.
5. **Webhook** is signed (HMAC over the payload) and POSTed to the merchant's
   configured URL. Failed deliveries are retried with exponential back-off.
6. The merchant can later call `POST /api/v1/merchant/withdrawals` to move
   funds out to an external address.

Public-facing payment status endpoint (no auth, used by the customer page):
`GET /api/v1/public/payments/{id}`.

---

## Production deployment

Production-oriented assets live under [`deploy/`](deploy/):

- `deploy/docker-compose.prod.yml` – Postgres + Redis + API + Nginx +
  Prometheus + Grafana + Loki
- `deploy/production-setup.sh` – host-side bootstrap script
- `nginx.prod.conf` – reverse-proxy configuration

Recommended hardening:
- Provide all secrets via your secrets manager — not via `.env` on disk.
- Use private RPC nodes (Infura, QuickNode, Helius, your own full node).
- Run Postgres with `sslmode=require` and connection pooling.
- Put the API behind a TLS-terminating reverse proxy (Nginx / Caddy).

---

## Contributing

Contributions are welcome. Please read [CONTRIBUTING.md](CONTRIBUTING.md)
before opening a PR. By contributing you agree that your code will be
licensed under the MIT License.

## Support

- Issues / feature requests: <https://github.com/Parikalp-Bhardwaj/zabypay/issues>
- Security disclosures (please do **not** open a public issue):
  [parikalp.123@gmail.com](mailto:parikalp.123@gmail.com)

---

## License

[MIT](LICENSE) © [Parikalp Bhardwaj](https://github.com/Parikalp-Bhardwaj) and contributors
