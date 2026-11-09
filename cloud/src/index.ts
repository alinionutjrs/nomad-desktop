import { serve } from "@hono/node-server";
import { neon } from "@neondatabase/serverless";
import { createRemoteJWKSet, jwtVerify } from "jose";
import { Hono, type Context } from "hono";
import { cors } from "hono/cors";
import Stripe from "stripe";

const env = (name: string, optional = false) => {
  const value = process.env[name]?.trim();
  if (!value && !optional) throw new Error(`Missing required environment variable: ${name}`);
  return value ?? "";
};

const sql = neon(env("DATABASE_URL"));
const stripe = new Stripe(env("STRIPE_SECRET_KEY"));
const app = new Hono();
const jwks = createRemoteJWKSet(new URL(env("AUTH_JWKS_URL")));
const allowedOrigins = new Set(["tauri://localhost", "http://tauri.localhost", "https://tauri.localhost", ...env("APP_ALLOWED_ORIGINS", true).split(",").map((item) => item.trim()).filter(Boolean)]);
app.use("/v1/*", cors({ origin: (origin) => allowedOrigins.has(origin) ? origin : null, allowHeaders: ["Authorization", "Content-Type"], allowMethods: ["GET", "POST", "OPTIONS"], maxAge: 600 }));

type User = { id: string; email?: string };
async function requireUser(authorization: string | undefined): Promise<User> {
  if (!authorization?.startsWith("Bearer ")) throw new Error("Authentication required");
  const { payload } = await jwtVerify(authorization.slice(7), jwks, {
    issuer: env("AUTH_ISSUER"), audience: env("AUTH_AUDIENCE"),
  });
  if (typeof payload.sub !== "string") throw new Error("Token has no subject");
  return { id: payload.sub, email: typeof payload.email === "string" ? payload.email : undefined };
}

function protect(handler: (user: User, context: Context) => Promise<Response>) {
  return async (context: any) => {
    try { return await handler(await requireUser(context.req.header("Authorization")), context); }
    catch (error) { return context.json({ error: error instanceof Error ? error.message : "Unauthorized" }, 401); }
  };
}

async function upsertUser(user: User) {
  await sql`INSERT INTO nomad_users(id,email) VALUES(${user.id},${user.email ?? null})
    ON CONFLICT(id) DO UPDATE SET email=COALESCE(EXCLUDED.email,nomad_users.email)`;
  const welcomeCredits = Number(process.env.FREE_STARTER_CREDITS ?? "100");
  if (Number.isSafeInteger(welcomeCredits) && welcomeCredits > 0) {
    await sql`INSERT INTO nomad_credit_ledger(user_id,delta,reason,source_id) VALUES(${user.id},${welcomeCredits},'free_starter','welcome:' || ${user.id}) ON CONFLICT(source_id) DO NOTHING`;
  }
}

app.get("/health", (c) => c.json({ ok: true, service: "nomad-cloud", billingConfigured: Boolean(process.env.STRIPE_SECRET_KEY && process.env.STRIPE_WEBHOOK_SECRET) }));

app.get("/v1/account", protect(async (user, c) => {
  await upsertUser(user);
  const [subscription] = await sql`SELECT status, current_period_end FROM nomad_subscriptions
    WHERE user_id=${user.id} ORDER BY updated_at DESC LIMIT 1`;
  const [credits] = await sql`SELECT COALESCE(SUM(delta),0)::int AS balance FROM nomad_credit_ledger WHERE user_id=${user.id}`;
  return c.json({ user: { id: user.id, email: user.email ?? null }, subscription: subscription ?? null, credits: credits?.balance ?? 0, plan: subscription?.status === "active" || subscription?.status === "trialing" ? "pro" : "free" });
}));

app.post("/v1/billing/checkout", protect(async (user, c) => {
  const body = await c.req.json<{ interval?: string }>().catch(() => ({} as { interval?: string }));
  const price = body.interval === "year" ? env("STRIPE_PRICE_PRO_YEARLY", true) : env("STRIPE_PRICE_PRO_MONTHLY", true);
  if (!price) return c.json({ error: "Billing is not configured for this interval" }, 503);
  await upsertUser(user);
  const [record] = await sql`SELECT stripe_customer_id FROM nomad_users WHERE id=${user.id}`;
  let customerId = record?.stripe_customer_id as string | undefined;
  if (!customerId) {
    const customer = await stripe.customers.create({ email: user.email, metadata: { nomad_user_id: user.id } }, { idempotencyKey: `customer:${user.id}` });
    customerId = customer.id;
    await sql`UPDATE nomad_users SET stripe_customer_id=${customerId} WHERE id=${user.id}`;
  }
  const returnUrl = env("APP_RETURN_URL");
  const session = await stripe.checkout.sessions.create({
    mode: "subscription", customer: customerId, line_items: [{ price, quantity: 1 }],
    success_url: `${returnUrl}?billing=success`, cancel_url: `${returnUrl}?billing=cancelled`,
    client_reference_id: user.id, subscription_data: { metadata: { nomad_user_id: user.id } },
  }, { idempotencyKey: `checkout:${user.id}:${body.interval === "year" ? "year" : "month"}:${Math.floor(Date.now() / 60000)}` });
  return c.json({ url: session.url });
}));

app.post("/v1/billing/portal", protect(async (user, c) => {
  const [record] = await sql`SELECT stripe_customer_id FROM nomad_users WHERE id=${user.id}`;
  if (!record?.stripe_customer_id) return c.json({ error: "No billing customer exists for this account" }, 404);
  const portal = await stripe.billingPortal.sessions.create({ customer: record.stripe_customer_id as string, return_url: env("APP_RETURN_URL") });
  return c.json({ url: portal.url });
}));

app.post("/v1/credits/consume", protect(async (user, c) => {
  const body = await c.req.json<{ requestId?: string; units?: number }>().catch(() => ({} as { requestId?: string; units?: number }));
  const requestId = body.requestId?.trim(); const units = body.units;
  if (!requestId || requestId.length > 120 || !Number.isSafeInteger(units) || (units as number) < 1 || (units as number) > 100_000) return c.json({ error: "A valid requestId and units (1–100000) are required" }, 400);
  await upsertUser(user);
  const sourceId = `usage:${user.id}:${requestId}`;
  const existing = await sql`SELECT delta FROM nomad_credit_ledger WHERE source_id=${sourceId}`;
  if (existing.length) return c.json({ charged: Math.abs(Number(existing[0].delta)), idempotent: true });
  const inserted = await sql`INSERT INTO nomad_credit_ledger(user_id,delta,reason,source_id)
    SELECT ${user.id},-${units as number},'agent_usage',${sourceId}
    WHERE (SELECT COALESCE(SUM(delta),0) FROM nomad_credit_ledger WHERE user_id=${user.id}) >= ${units as number}
    ON CONFLICT(source_id) DO NOTHING RETURNING delta`;
  if (!inserted.length) {
    const [duplicate] = await sql`SELECT delta FROM nomad_credit_ledger WHERE source_id=${sourceId}`;
    if (duplicate) return c.json({ charged: Math.abs(Number(duplicate.delta)), idempotent: true });
    return c.json({ error: "Insufficient credits" }, 409);
  }
  const [balance] = await sql`SELECT COALESCE(SUM(delta),0)::int AS balance FROM nomad_credit_ledger WHERE user_id=${user.id}`;
  return c.json({ charged: units, balance: balance?.balance ?? 0 });
}));

app.post("/v1/stripe/webhook", async (c) => {
  const signature = c.req.header("stripe-signature");
  if (!signature) return c.text("Missing Stripe signature", 400);
  let event: Stripe.Event;
  try { event = stripe.webhooks.constructEvent(await c.req.text(), signature, env("STRIPE_WEBHOOK_SECRET")); }
  catch { return c.text("Invalid webhook signature", 400); }
  const [seen] = await sql`SELECT event_id FROM stripe_events WHERE event_id=${event.id}`;
  if (seen) return c.json({ received: true, duplicate: true });

  if (event.type === "customer.subscription.created" || event.type === "customer.subscription.updated" || event.type === "customer.subscription.deleted") {
    const subscription = event.data.object as Stripe.Subscription;
    const userId = subscription.metadata.nomad_user_id;
    const customerId = typeof subscription.customer === "string" ? subscription.customer : subscription.customer.id;
    if (userId) {
      await sql`INSERT INTO nomad_users(id,stripe_customer_id) VALUES(${userId},${customerId}) ON CONFLICT(id) DO UPDATE SET stripe_customer_id=EXCLUDED.stripe_customer_id`;
      await sql`INSERT INTO nomad_subscriptions(id,user_id,stripe_customer_id,price_id,status,current_period_end)
        VALUES(${subscription.id},${userId},${customerId},${subscription.items.data[0]?.price.id ?? "unknown"},${subscription.status},${new Date(subscription.items.data[0]?.current_period_end ? subscription.items.data[0].current_period_end * 1000 : Date.now()).toISOString()})
        ON CONFLICT(id) DO UPDATE SET status=EXCLUDED.status,price_id=EXCLUDED.price_id,current_period_end=EXCLUDED.current_period_end,updated_at=now()`;
    }
  }
  if (event.type === "invoice.paid") {
    const invoice = event.data.object as Stripe.Invoice;
    const subRef = invoice.parent?.subscription_details?.subscription;
    const subscriptionId = typeof subRef === "string" ? subRef : subRef?.id;
    if (subscriptionId) {
      let [subscription] = await sql`SELECT user_id FROM nomad_subscriptions WHERE id=${subscriptionId}`;
      if (!subscription) {
        const source = await stripe.subscriptions.retrieve(subscriptionId);
        const userId = source.metadata.nomad_user_id;
        if (userId) {
          const customerId = typeof source.customer === "string" ? source.customer : source.customer.id;
          await sql`INSERT INTO nomad_users(id,stripe_customer_id) VALUES(${userId},${customerId}) ON CONFLICT(id) DO UPDATE SET stripe_customer_id=EXCLUDED.stripe_customer_id`;
          await sql`INSERT INTO nomad_subscriptions(id,user_id,stripe_customer_id,price_id,status,current_period_end)
            VALUES(${source.id},${userId},${customerId},${source.items.data[0]?.price.id ?? "unknown"},${source.status},${new Date((source.items.data[0]?.current_period_end ?? Date.now() / 1000) * 1000).toISOString()})
            ON CONFLICT(id) DO UPDATE SET status=EXCLUDED.status,updated_at=now()`;
          [subscription] = await sql`SELECT user_id FROM nomad_subscriptions WHERE id=${subscriptionId}`;
        }
      }
      const monthlyCredits = Number(process.env.PRO_CREDITS_PER_BILLING_CYCLE ?? "1000");
      if (subscription?.user_id && Number.isSafeInteger(monthlyCredits) && monthlyCredits > 0) {
        await sql`INSERT INTO nomad_credit_ledger(user_id,delta,reason,source_id) VALUES(${subscription.user_id as string},${monthlyCredits},'subscription_invoice',${`invoice:${invoice.id}`}) ON CONFLICT(source_id) DO NOTHING`;
      }
    }
  }
  await sql`INSERT INTO stripe_events(event_id,event_type) VALUES(${event.id},${event.type}) ON CONFLICT(event_id) DO NOTHING`;
  return c.json({ received: true });
});

app.notFound((c) => c.json({ error: "Not found" }, 404));
const port = Number(process.env.PORT ?? 8787);
serve({ fetch: app.fetch, port, hostname: "0.0.0.0" });
console.log(`Nomad cloud API listening on ${port}`);
