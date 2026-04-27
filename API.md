# ZabyPay — API Reference

End-to-end reference for the ZabyPay public API. Walks the full lifecycle:
**register → login → create merchant → create API key → create payment →
check balances → withdraw**.

- Base URL (default): `http://localhost:8080`
- All routes below are prefixed with `/api/v1` unless stated otherwise.
- Supported chains in this build: **ETH, BNB, SOL** (native currencies only).
  Bitcoin and USDT code paths exist in the source tree but are rejected at
  the API boundary.

## Conventions

| Concept | Header / format |
|---|---|
| User auth (dashboard) | `Authorization: Bearer <JWT>` |
| Merchant auth (server-to-server) | `X-API-Key: <api_key>` |
| Content type for `POST` / `PUT` | `Content-Type: application/json` |
| Errors | JSON body with `message` / `error`; non-2xx HTTP code |
| Timestamps | ISO-8601 UTC (e.g. `2026-04-26T12:33:58.408729Z`) |
| Amounts | strings with full precision (e.g. `"0.01"`) |

---

## 0. Health

### `GET /health`
Liveness probe used by Docker / load balancers. **No auth.**

```bash
curl http://localhost:8080/health
```
**Response — 200**
```json
{ "status": "ok", "timestamp": "2026-04-26T12:33:58Z", "version": "0.1.0" }
```

---

## 1. User auth

### 1.1 `POST /api/v1/client/register`
Creates a new user. **No email verification** — the row is written directly
into the `user` table with `emailverified=true, is_active=true`.

**Request**
```bash
curl -X POST http://localhost:8080/api/v1/client/register \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Acme",
    "email": "you@example.com",
    "password": "supersecret",
    "confirm_password": "supersecret"
  }'
```
**Response — 201 Created**
```json
{
  "status_code": 201,
  "message": "User registered successfully. You can log in immediately.",
  "success": true
}
```
**Errors**
| Code | When |
|---|---|
| 400 | Passwords do not match · invalid email · password < 8 chars · `User already exists` |

---

### 1.2 `POST /api/v1/client/login`
Returns a JWT (24 h TTL).

**Request**
```bash
curl -X POST http://localhost:8080/api/v1/client/login \
  -H "Content-Type: application/json" \
  -d '{"email":"you@example.com","password":"supersecret"}'
```
**Response — 200**
```json
{
  "status_code": 200,
  "message": "Login successful",
  "success": true,
  "token": "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9..."
}
```
Save this as `JWT=…` and put it in `Authorization: Bearer $JWT` for every
endpoint marked **(JWT)** below.

**Errors**
| Code | When |
|---|---|
| 400 | `Invalid credentials` (unknown email or wrong password) |

---

### 1.3 `POST /api/v1/client/test-login`
Same behaviour as `/login` (kept for back-compat with older clients).

---

> The previous `POST /api/v1/client/verify` endpoint has been **removed**.
> Calling it returns `404 Not Found`.

---

## 2. User wallets (JWT)

### 2.1 `POST /api/v1/client/wallet/generate-wallet`
Generates an HD-derived deposit address for the logged-in user.

**Request**
```bash
curl -X POST http://localhost:8080/api/v1/client/wallet/generate-wallet \
  -H "Authorization: Bearer $JWT" \
  -H "Content-Type: application/json" \
  -d '{"currency":"eth"}'        # eth | bnb | sol
```
**Response — 200**
```json
{
  "address": "0x9aBc...e123",
  "currency": "eth",
  "created_at": "2026-04-26T12:35:01Z"
}
```

### 2.2 `GET /api/v1/client/wallet/wallets`
Lists all wallets owned by the user.

**Response — 200**
```json
{
  "user": {
    "id": "cc45ddf7-...", "name": "Acme",
    "email": "you@example.com", "created_at": "2026-04-26T12:33:58Z"
  },
  "wallets": [
    { "id": "...", "currency": "eth", "public_key": "0x9aBc...", "created_at": "..." },
    { "id": "...", "currency": "sol", "public_key": "8xT...",    "created_at": "..." }
  ],
  "total_wallets": 2
}
```

---

## 3. Merchants (JWT)

### 3.1 `POST /api/v1/merchant/merchants`
Creates a merchant under the logged-in user.

**Request**
```bash
curl -X POST http://localhost:8080/api/v1/merchant/merchants \
  -H "Authorization: Bearer $JWT" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Acme Store",
    "description": "Demo merchant",
    "website_url": "https://acme.example.com",
    "webhook_url": "https://acme.example.com/zabypay/webhook"
  }'
```
**Response — 201**
```json
{
  "id": "8e7a1c2b-3d4f-49ab-9c01-abcdef012345",
  "name": "Acme Store",
  "description": "Demo merchant",
  "website_url": "https://acme.example.com",
  "webhook_url": "https://acme.example.com/zabypay/webhook",
  "environment_type": "mainnet",
  "preferred_networks": ["eth", "bnb", "sol"],
  "is_active": true,
  "created_at": "2026-04-26T12:36:10Z"
}
```
Save `id` as `MERCHANT_ID`.

### 3.2 List / get / update / delete

| Method | Path |
|---|---|
| `GET`    | `/api/v1/merchant/merchants` |
| `GET`    | `/api/v1/merchant/merchants/{id}` |
| `PUT`    | `/api/v1/merchant/merchants/{id}` (body = `UpdateMerchantRequest`) |
| `DELETE` | `/api/v1/merchant/merchants/{id}` |

### 3.3 Network metadata

| Method | Path | Returns |
|---|---|---|
| `GET` | `/api/v1/merchant/networks/supported`  | List of supported chain IDs / labels |
| `GET` | `/api/v1/merchant/networks/currencies` | List of currencies the merchant can accept |

---

## 4. API keys (JWT)

### 4.1 `POST /api/v1/merchant/merchants/{merchant_id}/api-keys`
Issues a fresh API + secret key. **The full keys are returned only here**;
later list calls only show the prefix.

**Request**
```bash
curl -X POST http://localhost:8080/api/v1/merchant/merchants/$MERCHANT_ID/api-keys \
  -H "Authorization: Bearer $JWT" \
  -H "Content-Type: application/json" \
  -d '{"name":"prod-key","environment_type":"mainnet"}'
```
**Response — 201**
```json
{
  "id": "ak_01HX...",
  "name": "prod-key",
  "api_key": "pk_live_8d2b9e7a4f1c....",
  "secret_key": "sk_live_61c0b3e2f8a9....",
  "environment_type": "mainnet",
  "is_active": true,
  "created_at": "2026-04-26T12:37:00Z"
}
```
Save `api_key` as `API_KEY`.

### 4.2 Other key endpoints

| Method | Path | Notes |
|---|---|---|
| `GET`    | `/api/v1/merchant/merchants/{id}/api-keys` | List (no full key returned) |
| `GET`    | `/api/v1/merchant/merchants/{id}/api-keys/{key_id}/reveal` | Re-reveal full key once |
| `DELETE` | `/api/v1/merchant/merchants/{id}/api-keys/{key_id}` | Revoke |

---

## 5. Payments (X-API-Key)

### 5.1 `POST /api/v1/payments/payment-requests`
Creates a payment request and a fresh receiving address. Customer scans the
returned `payment_uri` (BIP-21 / EIP-681 / Solana Pay) to pay.

**Request**
```bash
curl -X POST http://localhost:8080/api/v1/payments/payment-requests \
  -H "X-API-Key: $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "amount": "0.00043",
    "currency": "eth",                 # eth | bnb | sol
    "external_id": "order-1001",
    "expires_in_seconds": 3600,
    "metadata": { "order_ref": "1001" }
  }'
```
**Response — 201**
```json
{
  "id": "pr_a1b2c3d4-...",
  "merchant_id": "8e7a1c2b-...",
  "external_id": "order-1001",
  "amount": "0.01",
  "currency": "ETHEREUM",
  "wallet_address": "0xC0fFee0000000000000000000000000000abcdef",
  "status": "pending",
  "metadata": { "order_ref": "1001" },
  "payment_url": "http://localhost:3000/pay/pr_a1b2c3d4-...",
  "expires_at": "2026-04-26T13:37:00Z",
  "paid_at": null,
  "created_at": "2026-04-26T12:37:00Z",
  "updated_at": "2026-04-26T12:37:00Z",
  "qr_data": {
    "payment_uri": "ethereum:0xC0fFee...@1?value=10000000000000000",
    "amount": "0.01",
    "address": "0xC0fFee...",
    "currency": "ETHEREUM"
  },
  "usd_price": "3120.45",
  "usd_value": "31.20",
  "price_snapshot_ts": "2026-04-26T12:37:00Z",
  "is_simulated_price": false
}
```

> Render the `qr_data.payment_uri` string with any QR library, or feed it
> directly into a wallet app. The customer doesn't need any custom frontend:
> see **[PAYING.md](PAYING.md)** for a step-by-step guide (installing
> `qrencode` on macOS / Ubuntu, browser fallback, and pasting the URI
> straight into MetaMask / Trust Wallet / Phantom).

### 5.2 Other payment endpoints

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `GET`  | `/api/v1/payments/payment-requests` | API key | List (filters: `page,limit,status,currency,from_date,to_date,environment`) |
| `GET`  | `/api/v1/payments/payment-requests/{payment_source_id}` | API key | Get one |
| `PUT`  | `/api/v1/payments/payment-requests/{id}/status` | API key | Manual status override |
| `POST` | `/api/v1/payments/payment-requests/{id}/confirm` | API key | Manual confirm with `transaction_hash` |
| `GET`  | `/api/v1/public/payments/{id}` | none | Read-only view for the customer page |
| `GET`  | `/api/v1/merchant/payment-requests` | JWT | Same as 5.2 list, dashboard variant |
| `GET`  | `/api/v1/merchant/payment-requests/{id}` | JWT | Dashboard get-one |

### 5.3 Webhooks emitted (server → merchant `webhook_url`)

```json
{
  "event": "payment.paid",
  "data": { /* PaymentResponse */ },
  "timestamp": "2026-04-26T12:42:11Z",
  "signature": "hex(hmac_sha256(secret_key, body))"
}
```
Verify by recomputing the HMAC with `secret_key` over the raw request body.

| Event | When |
|---|---|
| `payment.created` | Right after creation |
| `payment.paid`    | First confirmation reached |
| `payment.expired` | `expires_at` passed without payment |
| `payment.failed`  | Internal failure path |

### 5.4 Webhook management (JWT)

| Method | Path |
|---|---|
| `GET`  | `/api/v1/webhooks/merchants/{merchant_id}/webhooks` |
| `GET`  | `/api/v1/webhooks/webhooks/{id}` |
| `POST` | `/api/v1/webhooks/webhooks/{id}/resend` |
| `GET`  | `/api/v1/webhooks/merchants/{merchant_id}/webhooks/stats` |
| `POST` | `/api/v1/webhooks/webhooks/test` |

### 5.5 Manual on-chain verification

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/api/v1/verify/payment` | Verify a tx by hash |
| `GET`  | `/api/v1/verify/payment/{id}` | Force re-check this payment |
| `GET`  | `/api/v1/verify/status/{id}` | Last known status |
| `GET`  | `/api/v1/verify/explorer` | Block-explorer link helper |
| `POST` | `/api/v1/verify/manual` | Mark paid by tx hash |

---

### 5.6 Recovering a `payment_id` (and `payment_url`) you've lost

The `id` and `payment_url` come back **once**, in the response to
`POST /api/v1/payments/payment-requests` (§5.1). If you didn't store them —
or you need to find a payment whose status is `failed` / `expired` /
`pending` — use one of these.

> Common confusion: **paid** payments also surface as
> `wallet_breakdown[*].payment_source_id` in `GET /api/v1/merchant/balances`
> (§6.1). That field IS the `payment_request.id` for the deposit that
> funded that wallet. **Failed / expired / unpaid** payments don't credit a
> wallet, so they won't appear in balances — use the methods below.

#### A. List recent payments (filter by status)

```bash
# failed
curl -s -H "X-API-Key: $API_KEY" \
  "http://localhost:8080/api/v1/payments/payment-requests?status=failed&limit=50&environment=mainnet" \
  | jq '.payments[] | {id, external_id, status, amount, wallet_address, payment_url, created_at}'

# expired
curl -s -H "X-API-Key: $API_KEY" \
  "http://localhost:8080/api/v1/payments/payment-requests?status=expired&limit=50&environment=mainnet" \
  | jq '.payments[] | {id, external_id, status, payment_url, created_at}'

# everything (no status filter)
curl -s -H "X-API-Key: $API_KEY" \
  "http://localhost:8080/api/v1/payments/payment-requests?limit=50&environment=mainnet" \
  | jq '.payments[] | {id, external_id, status, payment_url, created_at}'
```

Server-side filters supported on this endpoint: `page`, `limit`, `status`,
`currency`, `from_date`, `to_date`, `environment`. There is **no** server-side
filter for `external_id` or `wallet_address` — fetch and filter with `jq`
(B / C below).

#### B. Find by your own `external_id`

```bash
EXT_ID="order-1001"
curl -s -H "X-API-Key: $API_KEY" \
  "http://localhost:8080/api/v1/payments/payment-requests?limit=100&environment=mainnet" \
  | jq --arg e "$EXT_ID" \
       '.payments[] | select(.external_id == $e) | {id, status, payment_url}'
```

#### C. Find by the wallet address that's on the QR

```bash
ADDR="0xafc9177d4455ba0788c45f1bca94e2ddd248ab08"
curl -s -H "X-API-Key: $API_KEY" \
  "http://localhost:8080/api/v1/payments/payment-requests?limit=100&environment=mainnet" \
  | jq --arg a "$ADDR" \
       '.payments[] | select((.wallet_address|ascii_downcase) == ($a|ascii_downcase))
                    | {id, status, payment_url, external_id}'
```

#### D. Dashboard variant (JWT instead of API key)

```bash
curl -s -H "Authorization: Bearer $JWT" \
  "http://localhost:8080/api/v1/merchant/payment-requests?status=failed" \
  | jq '.payments[] | {id, external_id, status, payment_url}'
```

#### E. From the webhook log (if `webhook_url` is configured)

Every webhook delivery — successful or not — was logged with the
originating `payment_request_id`:

```bash
curl -s -H "Authorization: Bearer $JWT" \
  "http://localhost:8080/api/v1/webhooks/merchants/$MERCHANT_ID/webhooks" \
  | jq '.[] | {payment_request_id, event, status, attempts, last_error, created_at}'
```

#### F. Direct DB read (last resort, full visibility)

```bash
psql "$DATABASE_URL" -x -c "
  SELECT id, external_id, status, currency, amount,
         wallet_address, payment_url, created_at, expires_at, paid_at
  FROM payment_request
  WHERE merchant_id = '$MERCHANT_ID'
  ORDER BY created_at DESC
  LIMIT 20;"
```

#### Once you have the `id`, fetch the full payload

```bash
PAYMENT_ID=<id-from-any-of-the-above>

# API-key auth (full payload)
curl -s -H "X-API-Key: $API_KEY" \
  http://localhost:8080/api/v1/payments/payment-requests/$PAYMENT_ID | jq .

# Or public read (no header — works in a browser)
curl -s http://localhost:8080/api/v1/public/payments/$PAYMENT_ID | jq .
```

Both responses include the `payment_url` field
(e.g. `http://localhost:3000/pay/<id>`).

> **Heads-up about `failed`:** in this build a payment moves to `failed`
> only via an explicit status update (`PUT /payment-requests/{id}/status`)
> or an internal failure path. The most common "I scanned the QR, it
> didn't work" outcome is actually `expired` (timer elapsed) or stays
> `pending` (tx never broadcast or got dropped). When hunting, check
> **both** `status=failed` **and** `status=expired`.

> **REST-client gotcha:** `X-API-Key` is a **custom request header**, not
> a Bearer token. In Postman / Insomnia / Bruno: leave the **Auth** tab
> on `None` and add `X-API-Key: <api_key>` under **Headers**. Setting a
> "Bearer Token Prefix" of `X-API-Key` produces
> `Authorization: X-API-Key …` which is **not** what the server reads
> and will return `401 "Unauthorized"`.

---

## 6. Balances (JWT)

### 6.1 `GET /api/v1/merchant/balances?environment=mainnet`

**Response — 200**
```json
{
  "environment": "mainnet",
  "balances": [
    {
      "currency": "ethereum", "network": "eth", "symbol": "ETH",
      "available": "0.4530", "pending": "0.0000", "total": "0.4530",
      "decimals": 18, "updated_at": "2026-04-26T12:45:00Z"
    },
    {
      "currency": "bnb", "network": "bnb", "symbol": "BNB",
      "available": "1.2000", "pending": "0.0000", "total": "1.2000",
      "decimals": 18, "updated_at": "2026-04-26T12:45:00Z"
    },
    {
      "currency": "solana", "network": "sol", "symbol": "SOL",
      "available": "12.50", "pending": "0.00", "total": "12.50",
      "decimals": 9, "updated_at": "2026-04-26T12:45:00Z"
    }
  ],
  "wallet_breakdown": [
    {
      "wallet_id": "wl_...", "wallet_address": "0xC0fFee...",
      "currency": "ethereum", "network": "eth",
      "available_balance": "0.0100", "usd_value": "31.20",
      "payment_source_id": "pr_a1b2c3d4-...",
      "created_at": "2026-04-26T12:37:00Z",
      "last_transaction_at": "2026-04-26T12:42:11Z"
    }
  ],
  "total_usd_value": "1452.78",
  "total_wallets": 5,
  "last_updated": "2026-04-26T12:45:00Z"
}
```

### 6.2 Other balance endpoints

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/v1/merchant/multi-wallet/balances` | Aggregated per-wallet view |
| `GET` | `/api/v1/merchant/fee-estimate?network=eth&amount=0.1&to_address=0x...` | Pre-flight fee preview |
| `GET` | `/api/v1/merchant/max-withdrawable?network=eth` | Max amount you could send right now |

**`fee-estimate` response — 200**
```json
{
  "network": "eth", "currency": "ethereum",
  "balance": "0.4530",
  "estimated_fee": "0.00041",
  "transferable": "0.4523",
  "net_amount": "0.0996",
  "decimals": 18,
  "gas_price": "21000000000",
  "gas_limit": "21000",
  "fee_rate": null,
  "priority": "medium",
  "estimated_confirmation_time": "2-5 minutes",
  "safety_buffer": "0.0001"
}
```

**`max-withdrawable` response — 200**
```json
{
  "network": "eth", "currency": "ethereum",
  "max_withdrawable": "0.4523",
  "total_balance": "0.4530",
  "estimated_fee": "0.00041",
  "gas_price": "21000000000",
  "gas_limit": "21000",
  "wallets_available": 3
}
```

---

## 7. Withdrawals (JWT)

End-to-end flow:
**1.** check `max-withdrawable` →
**2.** (optional) `fee-estimate` for a specific amount →
**3.** `POST /merchant/withdrawals` →
**4.** track via `GET /merchant/withdrawals/{id}`.

> **Auth:** every withdrawal endpoint is **JWT-only**. API keys grant
> payment-creation, not money-out.

### 7.1 Pre-flight 1 — `GET /api/v1/merchant/max-withdrawable`

Tells you the largest amount the gateway will let you send right now after
gas + safety buffer. **Always call this first** so you don't ask for more
than the wallet can cover.

**Request**
```bash
curl -s -H "Authorization: Bearer $JWT" \
  "http://localhost:8080/api/v1/merchant/max-withdrawable?network=eth&environment=mainnet" \
  | jq .
```

**Response — 200**
```json
{
  "network": "eth",
  "currency": "ethereum",
  "max_withdrawable": "0.00042482",
  "total_balance": "0.00043",
  "estimated_fee": "0.00000254174982705",
  "gas_price": "20000000000",
  "gas_limit": "21000",
  "wallets_available": 1
}
```

Save the value:
```bash
MAX=$(curl -s -H "Authorization: Bearer $JWT" \
  "http://localhost:8080/api/v1/merchant/max-withdrawable?network=eth&environment=mainnet" \
  | jq -r .max_withdrawable)
echo "MAX = $MAX"      # MAX = 0.00042482
```

### 7.2 Pre-flight 2 — `GET /api/v1/merchant/fee-estimate` (optional)

Use this when you want to send a **specific** amount and need to confirm
the gas cost before submitting.

**Request**
```bash
DEST='0xE5a1Cc82351CE032bEF1e9707dA37b8bc11B01ff'

curl -s -H "Authorization: Bearer $JWT" \
  "http://localhost:8080/api/v1/merchant/fee-estimate?network=eth&environment=mainnet&amount=0.0001&to_address=$DEST" \
  | jq .
```

**Response — 200**
```json
{
  "network": "eth",
  "currency": "ethereum",
  "balance": "0.00043",
  "estimated_fee": "0.00000254174982705",
  "transferable": "0.00042482",
  "net_amount": "0.00009746",
  "decimals": 18,
  "gas_price": "20000000000",
  "gas_limit": "21000",
  "fee_rate": null,
  "priority": "medium",
  "estimated_confirmation_time": "2-5 minutes",
  "safety_buffer": "0.0001"
}
```

**If you ask for too much** the response is a 400:
```text
"Insufficient transferable balance. Maximum transferable: 0.000009 ETH, Requested: 0.00043 ETH"
```
Lower `amount` (or use the value of `transferable`) and retry.

---

### 7.3 `POST /api/v1/merchant/withdrawals` — create + broadcast

**Body fields**

| Field | Required | What it does |
|---|---|---|
| `environment` | yes | `mainnet` or `testnet` (must match the funded wallet) |
| `network` | yes | `eth` · `bnb` · `sol` (only these in this build) |
| `to_address` | yes | Recipient — validated for the chain (10–100 chars) |
| `amount` | yes | Decimal string, e.g. `"0.00042482"`. **Use the value from `max-withdrawable`** — see warning at the bottom about `"max"`. |
| `amount_type` | no | `"crypto"` (default) or `"usd"` (gateway converts at current price) |
| `idempotency_key` | yes | Unique per request. Re-using a value returns the original withdrawal instead of creating a duplicate. |
| `external_id` | no | Your own payout reference |
| `wallet_selection_strategy` | no | `optimal` (default) · `fifo` · `largest_first` (multi-wallet only) |

**Request**
```bash
DEST='0xE5a1Cc82351CE032bEF1e9707dA37b8bc11B01ff'
MAX='0.00042482'                       # from §7.1

curl -i -X POST http://localhost:8080/api/v1/merchant/withdrawals \
  -H "Authorization: Bearer $JWT" \
  -H "Content-Type: application/json" \
  -d "{
    \"environment\":\"mainnet\",
    \"network\":\"eth\",
    \"to_address\":\"$DEST\",
    \"amount\":\"$MAX\",
    \"amount_type\":\"crypto\",
    \"idempotency_key\":\"payout-$(date +%s)\",
    \"external_id\":\"payout-1\"
  }"
```

**Response — 201 Created** (real successful response, formatted):
```json
{
  "id": "23d1a19e-9a6d-49a7-8400-aa59ff88b448",
  "merchant_id": "387aaa65-fcd2-40b9-9ccd-735df764523a",
  "external_id": "23d1a19e-9a6d-49a7-8400-aa59ff88b448",
  "idempotency_key": "payout-1777199082",
  "environment": "mainnet",
  "network": "eth",
  "currency": "ETH",
  "to_address": "0xE5a1Cc82351CE032bEF1e9707dA37b8bc11B01ff",
  "amount_type": "crypto",
  "requested_amount": "0.00042482",
  "transferred_amount": "0.00042745825017295",
  "amount": "0.00042745825017295",
  "net_amount": "0.00042745825017295",
  "fee": "0.00000254174982705",
  "total_fee": "0.00000254174982705",
  "usd_equivalent": null,
  "status": "confirmed",
  "blockchain_confirmations": 1,
  "confirmations": 1,
  "required_confirmations": 12,
  "tx_hash": "0xcb49258da045bc3122e85ee9b3273fdcd86267de5e55f1263e14534bc00cc25b",
  "tx_hashes": ["0xcb49258da045bc3122e85ee9b3273fdcd86267de5e55f1263e14534bc00cc25b"],
  "explorer_url": "https://etherscan.io/tx/0xcb49258da045bc3122e85ee9b3273fdcd86267de5e55f1263e14534bc00cc25b",
  "explorer_urls": ["https://etherscan.io/tx/0xcb49258da045bc3122e85ee9b3273fdcd86267de5e55f1263e14534bc00cc25b"],
  "multi_wallet": true,
  "aggregation_method": "evm_deposit_discovery",
  "wallets_used": [
    {
      "wallet_id": "0200bc1a-5909-4e1b-a275-e8e2860a1abf",
      "address": "0x7e44c3cf4238ddd89ac67c91fccc6b9c3a0b8b17",
      "tx_hash": "0xcb49258da045bc3122e85ee9b3273fdcd86267de5e55f1263e14534bc00cc25b",
      "transferred_amount": "0x184c561286a16",
      "gas_cost_wei": "0x24fcc1875ea",
      "gas_topup_amount": "0x0",
      "gas_topup_tx_hash": null
    }
  ],
  "message": "Withdrawal completed using 1 ETH wallets via deposit aggregation",
  "broadcast_at": "2026-04-26T10:24:44.698983+00:00",
  "processed_at": "2026-04-26T10:24:44.698951+00:00",
  "confirmed_at": "2026-04-26T10:24:44.698954+00:00",
  "created_at":   "2026-04-26T10:24:44.698934+00:00",
  "updated_at":   "2026-04-26T10:24:44.698973+00:00"
}
```

Save the id:
```bash
WID=23d1a19e-9a6d-49a7-8400-aa59ff88b448
```

**Withdrawal status lifecycle:** `pending → broadcast → confirmed`
(or `failed` / `cancelled`).

**Field notes worth highlighting**
- `requested_amount` (what you asked for) ≠ `transferred_amount` (what
  actually went on-chain). The EVM aggregator picks the optimal sweep value,
  which can be slightly higher than `requested_amount` when the wallet only
  has one balance.
- `wallets_used[*].transferred_amount` and `gas_cost_wei` are **hex
  strings in wei**. Convert with
  `printf '%d\n' 0x184c561286a16` if you need decimals.
- For single-tx EVM withdrawals `tx_hash == tx_hashes[0]` and
  `explorer_url == explorer_urls[0]` (kept for backward compatibility).
- `usd_equivalent` is `null` when the price oracle returned no quote at
  send time — the on-chain transfer still succeeded.

### 7.4 Track confirmations — `GET /api/v1/merchant/withdrawals/{id}`

```bash
curl -s -H "Authorization: Bearer $JWT" \
  "http://localhost:8080/api/v1/merchant/withdrawals/$WID" \
  | jq '{status, blockchain_confirmations, required_confirmations, tx_hash, explorer_url}'
```

Auto-refresh every 5 s until confirmed:
```bash
watch -n 5 "curl -s -H 'Authorization: Bearer $JWT' \
  http://localhost:8080/api/v1/merchant/withdrawals/$WID \
  | jq '{status, blockchain_confirmations, required_confirmations}'"
```

Open `explorer_url` in your browser to watch the tx on Etherscan / BscScan /
Solscan.

### 7.5 List / cancel

| Method | Path | Notes |
|---|---|---|
| `GET`  | `/api/v1/merchant/withdrawals` | Paginated. Filters: `page,limit,status,network,environment,from_date,to_date` |
| `POST` | `/api/v1/merchant/withdrawals/{id}/cancel` | Cancel while still `pending`. Already-broadcast txs cannot be cancelled. |

**List request**
```bash
curl -s -H "Authorization: Bearer $JWT" \
  "http://localhost:8080/api/v1/merchant/withdrawals?status=confirmed&network=eth&limit=10" \
  | jq '.withdrawals[] | {id, status, amount, fee, tx_hash, created_at}'
```

**List response shape**
```json
{
  "withdrawals": [ /* WithdrawalResponse — see §7.3 */ ],
  "pagination": {
    "page": 1, "limit": 20, "total": 42, "total_pages": 3,
    "has_next": true, "has_prev": false
  },
  "summary": {
    "total_count": 42,
    "pending_count": 0, "processing_count": 1,
    "confirmed_count": 40, "failed_count": 1,
    "total_amount_usd": "12480.50",
    "total_fees_usd":  "9.81"
  }
}
```

### 7.6 Errors you'll actually see

| HTTP | Body | Cause / fix |
|---|---|---|
| 400 | `"Unsupported network: ... Supported networks: ETH, BNB, SOL (native currencies only)."` | Sent something other than `eth`/`bnb`/`sol`. |
| 400 | `"Bitcoin is not enabled in this build. Supported networks: ETH, BNB, SOL."` | Sent `network:"btc"`. |
| 400 | `"USDT is not enabled in this build. Supported networks: ETH, BNB, SOL."` | Sent `network:"usdt_erc20"` / `"usdt_bep20"`. |
| 400 | `"Insufficient transferable balance. Maximum transferable: X ETH, Requested: Y ETH"` | Hit on `fee-estimate` and on `POST /withdrawals`. Use `max-withdrawable` value. |
| 400 | `{ "error":"eth_withdrawal_failed", "code":"ETH_WITHDRAWAL_FAILED", "message":"... Validation error: Invalid amount format" }` | You sent `"amount":"max"` to an EVM withdrawal. The multi-wallet path doesn't resolve `"max"` — pass the explicit value from `max-withdrawable` instead. |
| 400 | empty body, `content-length: 0` | The `Authorization` header is malformed (e.g. JWT pasted across two terminal lines, embedded whitespace). Re-set `JWT` on a single line and retry. |
| 401 | `"Unauthorized"` | Missing / wrong / expired JWT. Re-login. |
| 409 | `{ "error":"duplicate_idempotency_key" }` | Re-using the same `idempotency_key`. Generate a new one (`uuidgen` or `payout-$(date +%s)`). |

### 7.7 Multi-wallet withdrawals (advanced)

Use these when a single deposit wallet can't cover the requested amount —
the gateway will sweep from multiple wallets in one logical withdrawal.

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/api/v1/merchant/multi-wallet/withdrawals/preview` | Dry-run: returns the `WithdrawalPlan` (which wallets, how much from each, fees, shortfall) |
| `POST` | `/api/v1/merchant/multi-wallet/withdrawals` | Execute the plan |
| `GET`  | `/api/v1/merchant/multi-wallet/withdrawals/{id}` | Inspect a multi-wallet withdrawal |

**Preview response — 200**
```json
{
  "withdrawal_id": "wd_preview_...",
  "requested_amount": "1.5",
  "amount_type": "crypto",
  "total_available": "1.62",
  "selected_wallets": [
    { "wallet_id": "wl_a", "wallet_address": "0x...", "available_balance": "1.0",
      "amount_to_transfer": "1.0", "estimated_fee": "0.00041",
      "net_amount": "0.99959", "order": 1 },
    { "wallet_id": "wl_b", "wallet_address": "0x...", "available_balance": "0.62",
      "amount_to_transfer": "0.5", "estimated_fee": "0.00041",
      "net_amount": "0.49959", "order": 2 }
  ],
  "total_fees_estimated": "0.00082",
  "net_transfer_amount": "1.49918",
  "can_fulfill": true,
  "shortfall_amount": null
}
```

---

## 8. End-to-end shell script

```bash
#!/usr/bin/env bash
set -euo pipefail
BASE=http://localhost:8080/api/v1
EMAIL="demo_$(date +%s)@example.com"
PASSWORD="123@Abcd"

# 1. register (no email verification)
curl -s -X POST $BASE/client/register \
  -H "Content-Type: application/json" \
  -d "{\"name\":\"Demo\",\"email\":\"$EMAIL\",
       \"password\":\"$PASSWORD\",\"confirm_password\":\"$PASSWORD\"}"

# 2. login → JWT
JWT=$(curl -s -X POST $BASE/client/login \
  -H "Content-Type: application/json" \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}" | jq -r .token)

# 3. create merchant
MERCHANT_ID=$(curl -s -X POST $BASE/merchant/merchants \
  -H "Authorization: Bearer $JWT" -H "Content-Type: application/json" \
  -d '{"name":"Acme"}' | jq -r .id)

# 4. create API key
API_KEY=$(curl -s -X POST $BASE/merchant/merchants/$MERCHANT_ID/api-keys \
  -H "Authorization: Bearer $JWT" -H "Content-Type: application/json" \
  -d '{"name":"prod","environment_type":"mainnet"}' | jq -r .api_key)

# 5. create a payment request (returns payment_uri for QR)
curl -s -X POST $BASE/payments/payment-requests \
  -H "X-API-Key: $API_KEY" -H "Content-Type: application/json" \
  -d '{"amount":"0.01","currency":"eth","external_id":"order-1"}'

# ... customer pays. Wait for webhook or poll /merchant/payment-requests ...

# 6. check balances
curl -s $BASE/merchant/balances -H "Authorization: Bearer $JWT"

# 7. withdraw
curl -s -X POST $BASE/merchant/withdrawals \
  -H "Authorization: Bearer $JWT" -H "Content-Type: application/json" \
  -d "{
    \"environment\":\"mainnet\",
    \"network\":\"eth\",
    \"to_address\":\"0xRecipient0000000000000000000000000000dead\",
    \"amount\":\"0.001\",
    \"amount_type\":\"crypto\",
    \"idempotency_key\":\"$(uuidgen)\"
  }"
```

---

## 9. Auth header cheat-sheet

| Endpoint family | Auth header |
|---|---|
| `/api/v1/client/register`, `/login`, `/test-login` | none |
| `/api/v1/client/wallet/*` | `Authorization: Bearer $JWT` |
| `/api/v1/merchant/*` (most) | `Authorization: Bearer $JWT` |
| `/api/v1/payments/*` | `X-API-Key: $API_KEY` |
| `/api/v1/public/payments/{id}` | none (customer-facing read) |
| `/api/v1/verify/*` | `X-API-Key: $API_KEY` |
| `/api/v1/webhooks/*` | `Authorization: Bearer $JWT` |
| `/health` | none |

---

## 10. Common errors

| HTTP | Body shape | Cause |
|---|---|---|
| 400 | `"Invalid credentials"` / `"User already exists"` / `"Passwords do not match"` | Auth-layer validation |
| 400 | `{"error":"validation","message":"..."}` | Body validation (missing fields, bad email, short password) |
| 400 | `"Unsupported network: ... Supported networks: ETH, BNB, SOL ..."` | Sending `btc` / `usdt_*` to a withdrawal/payment endpoint |
| 401 | `"Unauthorized"` | Missing / invalid JWT or API key |
| 404 | empty / `{"message":"Not found"}` | Removed routes (e.g. `/client/verify`) or unknown id |
| 409 | `{"error":"duplicate_idempotency_key"}` | Re-using an `idempotency_key` for a new withdrawal |
| 410 | (n/a) | Reserved for deprecated routes |
| 500 | `{"message":"..."}` | Server bug — file an issue with the request id from headers |
