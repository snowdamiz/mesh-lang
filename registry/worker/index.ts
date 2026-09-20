/// <reference types="@cloudflare/workers-types" />

// The registry is the same axum binary that ran on Fly: a long-lived HTTP
// service that needs Postgres and an S3 client, neither of which a Worker can
// hold. It runs as a Cloudflare Container, and this Worker is the front door
// that routes requests into it.
//
// Configuration reaches the container as environment variables. The container
// never sees the Worker's bindings, so every value the service reads from the
// environment has to be forwarded here: the plain ones from `vars` in
// wrangler.jsonc, the rest from `wrangler secret put`.
import { Container, getContainer } from "@cloudflare/containers";

interface Env {
  REGISTRY: DurableObjectNamespace<RegistryContainer>;
  // Secrets (wrangler secret put).
  DATABASE_URL: string;
  STORAGE_ACCESS_KEY_ID: string;
  STORAGE_SECRET_ACCESS_KEY: string;
  GITHUB_CLIENT_SECRET: string;
  SESSION_SECRET: string;
  // Plain configuration (wrangler.jsonc `vars`).
  STORAGE_ENDPOINT: string;
  STORAGE_BUCKET: string;
  STORAGE_REGION: string;
  GITHUB_CLIENT_ID: string;
  GITHUB_CALLBACK_URL: string;
  FRONTEND_URL: string;
}

export class RegistryContainer extends Container<Env> {
  // The axum service binds 0.0.0.0:$PORT, defaulting to 3000.
  defaultPort = 3000;
  // Sessions and uploads live in Postgres and R2, so an idle instance can be
  // reclaimed: the next request pays a cold start rather than a wrong answer.
  sleepAfter = "20m";

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    this.envVars = {
      DATABASE_URL: env.DATABASE_URL,
      STORAGE_ENDPOINT: env.STORAGE_ENDPOINT,
      STORAGE_BUCKET: env.STORAGE_BUCKET,
      STORAGE_REGION: env.STORAGE_REGION,
      STORAGE_ACCESS_KEY_ID: env.STORAGE_ACCESS_KEY_ID,
      STORAGE_SECRET_ACCESS_KEY: env.STORAGE_SECRET_ACCESS_KEY,
      GITHUB_CLIENT_ID: env.GITHUB_CLIENT_ID,
      GITHUB_CLIENT_SECRET: env.GITHUB_CLIENT_SECRET,
      GITHUB_CALLBACK_URL: env.GITHUB_CALLBACK_URL,
      SESSION_SECRET: env.SESSION_SECRET,
      FRONTEND_URL: env.FRONTEND_URL,
      PORT: String(this.defaultPort),
    };
  }
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    // One instance: the service is stateless (sessions are rows in Postgres),
    // but a single instance keeps one connection pool rather than one per
    // instance, which is what a free Neon branch can hold.
    return getContainer(env.REGISTRY).fetch(request);
  },
} satisfies ExportedHandler<Env>;
