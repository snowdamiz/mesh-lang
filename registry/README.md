# mesh-registry

The package registry behind `api.packages.meshlang.dev`: an axum service on
Postgres, with package tarballs in S3-compatible object storage.

It runs as a **Cloudflare Container**. `worker/index.ts` is the front door — a
Worker that routes every request into the container — and the container is the
same image that ran on Fly, built from `Dockerfile` unchanged.

Postgres is **Neon** (`mesh-registry` project). Cloudflare has no managed
Postgres, and the service uses sqlx, Postgres full-text search and a Postgres
session store, so the database stays off-platform. Object storage is
**Cloudflare R2** (`mesh-packages` bucket), reached over its S3 API — which is
what `src/storage/r2.rs` was already written against.

## Deploying

    npm install
    npx wrangler deploy

CI does this on every push to `main` (`.github/workflows/deploy-services.yml`).
`wrangler deploy` builds the Dockerfile and pushes it to Cloudflare's managed
registry, so the deploying machine needs Docker.

The service applies its own migrations at start-up (`sqlx::migrate!`), so there
is no separate migration step.

## Configuration

Non-secret values live in `wrangler.jsonc` under `vars`. The rest are Worker
secrets, set once per environment:

    npx wrangler secret put DATABASE_URL            # Neon connection string
    npx wrangler secret put SESSION_SECRET          # openssl rand -hex 32
    npx wrangler secret put GITHUB_CLIENT_SECRET    # GitHub OAuth app
    npx wrangler secret put STORAGE_ACCESS_KEY_ID   # R2 S3 API token
    npx wrangler secret put STORAGE_SECRET_ACCESS_KEY

`GITHUB_CLIENT_ID` is a `var`, not a secret, and is empty until the OAuth app
exists. The container reads all of these as environment variables; the Worker
forwards them in `RegistryContainer`'s constructor, so a value that is not
listed there never reaches the service.

R2 S3 credentials come from the Cloudflare dashboard (**R2 → API → Manage API
Tokens**); wrangler cannot mint them.

## The CI deploy token

`CLOUDFLARE_API_TOKEN` needs all of these. The zone-scoped Workers Routes
permission is easy to miss: without it the script uploads and the deploy then
fails on `/zones/<id>/workers/routes`, which looks like a deploy fault rather
than a missing permission.

| Scope | Permission |
|---|---|
| Account | Workers Scripts: Edit |
| Account | Workers R2 Storage: Edit |
| Account | Cloudflare Containers: Edit |
| Zone (`meshlang.dev`) | Workers Routes: Edit |
| Zone (`meshlang.dev`) | Zone: Read |

## Local development

`wrangler dev` runs the Worker and builds the container locally. Put secrets in
`.dev.vars` (git-ignored) rather than exporting them.

On a checkout that lives on exFAT, macOS writes an AppleDouble sidecar (`._*`)
beside every file, and Docker's build-context sender fails on their extended
attributes with `operation not permitted`. `.dockerignore` does not help — the
sender trips over them before the ignore rules apply. Delete them first:

    find . -name '._*' -delete
