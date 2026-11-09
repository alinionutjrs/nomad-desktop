CREATE TABLE IF NOT EXISTS nomad_users (
  id text PRIMARY KEY,
  email text,
  stripe_customer_id text UNIQUE,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS nomad_subscriptions (
  id text PRIMARY KEY,
  user_id text NOT NULL REFERENCES nomad_users(id) ON DELETE CASCADE,
  stripe_customer_id text NOT NULL,
  price_id text NOT NULL,
  status text NOT NULL,
  current_period_end timestamptz,
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS nomad_subscriptions_user_idx ON nomad_subscriptions(user_id);
CREATE TABLE IF NOT EXISTS nomad_credit_ledger (
  id bigserial PRIMARY KEY,
  user_id text NOT NULL REFERENCES nomad_users(id) ON DELETE CASCADE,
  delta integer NOT NULL,
  reason text NOT NULL,
  source_id text NOT NULL UNIQUE,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS stripe_events (
  event_id text PRIMARY KEY,
  event_type text NOT NULL,
  processed_at timestamptz NOT NULL DEFAULT now()
);
