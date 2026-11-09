# Nomad Cloud API

Optional account and billing service. The desktop application remains local-first when this service is not configured. This API deliberately does not simulate sign-in, paid entitlements, or credits when configuration is missing.

## Included routes

- `GET /health` — liveness and billing configuration status.
- `GET /v1/account` — verified identity, current subscription state, and ledger balance.
- `POST /v1/billing/checkout` — creates a Stripe-hosted subscription Checkout Session (`interval: month|year`).
- `POST /v1/billing/portal` — creates a Stripe Customer Portal session for the signed-in owner.
- `POST /v1/credits/consume` — atomically deducts usage from the ledger; idempotent by request ID and rejects insufficient balances.
- `POST /v1/stripe/webhook` — validates Stripe's signature against the unmodified request body; subscription state comes from Stripe events and paid invoice credits are idempotent.

The app's identity provider must issue signed JWTs matching `AUTH_ISSUER`, `AUTH_AUDIENCE`, and `AUTH_JWKS_URL`. The API only trusts the validated token subject; it never accepts a client-supplied user ID for billing. For Neon Auth, enable Auth on the intended project, use its issuer/JWKS metadata, and verify the resulting JWT claims before putting the values in the server environment. No existing Neon project is selected by this repository.

## Setup

1. Create a dedicated Neon project/branch and apply `migrations/001_billing.sql` with a migration user.
2. Create a Stripe product and recurring monthly/yearly Prices in test mode. Configure a webhook to `POST /v1/stripe/webhook` for `customer.subscription.created`, `customer.subscription.updated`, `customer.subscription.deleted`, and `invoice.paid`.
3. Copy `.env.example` to `.env` and set server-only credentials and Stripe Price IDs. Never place `DATABASE_URL`, `STRIPE_SECRET_KEY`, or `STRIPE_WEBHOOK_SECRET` in the desktop app or a frontend build.
4. Set `APP_RETURN_URL` to a registered desktop deep link or hosted return page. Add only the desktop/web origins you serve to `APP_ALLOWED_ORIGINS`.
5. Run `npm install`, `npm run build`, then `npm start` (or `npm run dev` for local development).

`FREE_STARTER_CREDITS` and `PRO_CREDITS_PER_BILLING_CYCLE` are server-side policy values, not UI promises. Paid credits are added from `invoice.paid`, never from the Checkout success redirect. The ledger retains immutable grant/usage rows; reconciliation and refund/chargeback policy should be added before launch. Use Stripe test mode and webhook forwarding for validation before switching to live keys.

## Deployment checklist

- Use TLS and secret storage in the hosting platform; keep the database role least-privileged.
- Verify the identity provider's exact issuer, audience, signing keys, and desktop return-link behavior.
- Configure Stripe Customer Portal and webhook retries, and reconcile subscription/invoice state.
- Add rate limits, observability with secret/PII redaction, privacy terms, tax settings, and support contact before public launch.
- Connect desktop account UI only after the identity sign-in flow and native return callback have been configured. No login or paid status is presented as active by the offline app.
