# Kraken API Key Generation Checklist

**Permissions required:** Query + Trade only. **Withdraw MUST be disabled.**

This is the canonical checklist for generating Kraken API keys that satisfy
MarketMarkovNet's threat model. Follow every step. Verify the final permissions
table before saving the key.

- Feature spec: `plans/market-markov-net/features/03 - kraken credentials and config.md`
- Env-var source of truth: `deploy/config.md`
- Global constraint: `.env` is gitignored; only `.env.example` is tracked.

## 1. Prerequisites

- Verified Kraken account (KYC complete, Intermediate or Pro tier).
- 2FA enabled (authenticator app — TOTP — not SMS).
- Funding for the trading account settled (deposit cleared and available).
- A host with a static egress IP (recommended for the IP allowlist in step 4).

## 2. UI walkthrough

1. Sign in to https://www.kraken.com/.
2. Navigate to **Account** → **Security** → **API** (legacy path) **or**
   **Settings** → **API** (newer path). Either is valid; the form fields are
   identical.
3. Click **"Add API key"** / **"Create key"**.
4. Enter a key description (key name): `marketmarkovnet` (or a descriptive
   project-scoped name such as `mmn-prod`).
5. Choose key type: **Standard API key**. Do **not** select a Master Key unless
   you have a specific, separate use case — Master Keys are not needed for
   REST trading and complicate revocation.
6. Set permissions per the matrix in section 3. **Uncheck Withdraw.**
7. (Recommended) Configure the IP allowlist per section 4.
8. Click **"Generate key"** / **"Create key"**. Kraken will display the API key
   and the API secret **once** — copy both immediately. The secret cannot be
   retrieved again.
9. Store the key and secret in `.env` at the project root (see section 5).
10. Acknowledge the 2FA prompt and complete key creation.

## 3. Permissions matrix

Kraken groups capabilities into several permission toggles. The names below
match the current Kraken UI. Tick only the rows marked **YES**; leave every
other toggle off.

| Permission                          | Required | Notes |
|-------------------------------------|----------|-------|
| Query Funds                         | YES      | Required for balance checks and position sizing. |
| Query Open Orders / Open Positions  | YES      | Required for the execution layer to reconcile state. |
| Query Closed Orders / Trade History | YES      | Required for fill reconciliation and PnL bookkeeping. |
| Query Ledger Entries                | YES      | Required for fee and funding accounting. |
| Export Data (statements / ledgers)  | YES      | Required to download trade history for parity checks. |
| Place & Cancel Orders (Trade)       | YES      | Required for live order placement and cancellation. |
| Withdraw Funds                      | **NO**   | **MUST be disabled.** Withdraw is never required by the engine and grants lateral movement on compromise. |
| Withdraw to Whitelisted Address     | **NO**   | **MUST be disabled.** Same rationale as Withdraw Funds. |
| Create / Modify Staking             | **NO**   | Disable. Out of scope. |
| Sub-account transfer                | **NO**   | Disable. Out of scope. |

Verification step after key creation: re-open **Settings → API**, select the
new key, and confirm every row marked **NO** is unchecked before pasting the
key into `.env`.

## 4. IP allowlist (recommended)

If the engine runs on a VPS with a static public IP:

1. On the same API creation form (or via **Edit** on an existing key), expand
   the **"Restrict access"** / **"IP allowlist"** section.
2. Add the public IPv4 address of the engine host (e.g. `203.0.113.42/32`).
3. If the Python inference service is on a different host, add that public
   IP as well.
4. Save and re-confirm with 2FA.

If the host has a dynamic IP, either pin the IP at the firewall (preferred —
Kraken does not allow CIDR ranges narrower than `/32` for individual hosts but
does accept a small list) or skip the allowlist and rely on the secret alone.
Do not skip the allowlist silently — record the decision in
`deploy/PROVISIONING.md`.

## 5. Storage

- Copy the API key and secret to `.env` at the project root:
  ```
  KRAKEN_API_KEY=<key>
  KRAKEN_API_SECRET=<secret>
  ```
- Restrict permissions: `chmod 600 .env`.
- Verify `.env` is gitignored: `git check-ignore .env` should print `.env`.
- For Docker Compose, do **not** bake the secret into the image. Use one of:
  - A host-mounted `.env` file passed via `env_file:` in `compose.yaml`.
  - Docker secrets (`/run/secrets/kraken_key` / `/run/secrets/kraken_secret`)
    and a small wrapper script that exports them as `KRAKEN_API_KEY` /
    `KRAKEN_API_SECRET` before launching the engine.
  - An external secret manager (HashiCorp Vault, AWS Secrets Manager, etc.)
    surfaced via an init container.
- Never echo the secret into logs or crash reports. The engine must redact
  both variables in any structured log output.

## 6. Verification

Use Kraken's public REST endpoint to confirm the key works for the permissions
granted. `GetAccountBalance` is the cheapest probe and exercises the Query
Funds permission:

```bash
API_KEY="$KRAKEN_API_KEY"
API_SECRET="$KRAKEN_API_SECRET"
NONCE=$(date +%s%N)
POST="nonce=${NONCE}"
PATH_URL="/0/private/Balance"
SIGN=$(printf '%s' "${NONCE}${POST}" | \
  openssl dgst -sha512 -hmac "${API_SECRET}" -binary | \
  base64)
curl -s -X POST "https://api.kraken.com${PATH_URL}" \
  -H "API-Key: ${API_KEY}" \
  -H "API-Sign: ${SIGN}" \
  -d "${POST}"
```

Expected: HTTP 200 with a JSON object of asset balances. Any `EAPI:Invalid
key` / `EAPI:Invalid signature` response means the key or secret was
transcribed incorrectly; rotate and retry.

Kraken does not expose a withdraw probe via REST, so the absence of the
Withdraw permission must be verified in the UI only (see section 3). A
periodic manual review is acceptable; an automated check is not possible.

## 7. Revocation / rotation

- Rotate keys every 90 days. Mark the rotation date in
  `deploy/PROVISIONING.md` or the runbook.
- On any suspected leak (logs, screenshots, commit history, chat transcript):
  revoke immediately via **Settings → API → Edit → Delete key**, then issue a
  new key following this checklist from step 1.
- When the engine is decommissioned, revoke the key as part of the teardown
  procedure even if it is no longer in use.
- After revocation, the old key and secret are invalidated by Kraken; they can
  be removed from `.env` and from any Docker secrets store.
