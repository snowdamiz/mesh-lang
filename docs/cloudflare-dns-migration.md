# Moving meshlang.dev and its services onto Cloudflare

The services moved from Fly.io to Cloudflare. Workers custom domains only work
when the zone is on Cloudflare, so `meshlang.dev` has to move off Vercel DNS
(`ns1/ns2.vercel-dns.com`) for `packages.meshlang.dev` and
`api.packages.meshlang.dev` to resolve to the new Workers.

Until that happens the services are reachable at their `workers.dev` URLs and
`meshlang.dev` itself is untouched.

## What still needs a browser, and why

Everything else was done from the CLI. These four cannot be, and it is worth
recording why so nobody re-litigates it:

| Step | Why no CLI |
|---|---|
| Add the `meshlang.dev` zone | `wrangler login --scopes-list` offers `zone:read` only; there is no zone-write or zone-create scope, and the machine holds no other Cloudflare credential. |
| Change the nameservers | `meshlang.dev` is registered **at Vercel**, and `vercel domains` has no nameserver subcommand — only list/inspect/add/buy/move/transfer-in/renew. |
| Create the R2 S3 API token | `wrangler r2` has no token subcommand, and minting one through the API needs token-create permission the OAuth token does not carry. |
| Create the GitHub OAuth app | GitHub's REST API has no endpoint for creating OAuth apps; both `/applications` and `/user/applications` are 404. Only the GitHub App *manifest* flow exists, and it is also a browser redirect. |

Already done from the CLI, for contrast: the Neon project and database, the R2
bucket, both Workers, the `DATABASE_URL` and `SESSION_SECRET` secrets, and the
`CLOUDFLARE_ACCOUNT_ID` repository secret (`gh secret set`).

## Records that must survive the move

Only the apex is serving anything today: `meshlang.dev` is the documentation
site on GitHub Pages. Recreate these before changing nameservers, and set the
GitHub Pages records to **DNS only** (grey cloud) so Pages terminates TLS
itself.

| Type | Name | Value |
|---|---|---|
| A | `@` | `185.199.108.153` |
| A | `@` | `185.199.109.153` |
| A | `@` | `185.199.110.153` |
| A | `@` | `185.199.111.153` |
| CNAME | `www` | `snowdamiz.github.io` |
| TXT | `_github-pages-challenge-snowdamiz` | `c723932d186e25464ff8fd2acf20ba` |
| CAA | `@` | `0 issue "letsencrypt.org"` |
| CAA | `@` | `0 issue "pki.goog"` |
| CAA | `@` | `0 issue "sectigo.com"` |

The TXT record is GitHub's domain verification. Losing it unverifies the domain
for the account, so it is the one record whose absence is not obvious from a
browser.

The three CAA records restrict who may issue certificates. Cloudflare's
Universal SSL issues from Let's Encrypt and Google Trust Services, both already
allowed, so Worker custom domains can get certificates without changing them.

## Records that are deliberately not recreated

- `packages` A `66.241.125.44` and AAAA `2a09:8280:1::db:2b21:0` — the Fly app,
  which no longer exists.
- `api.packages` CNAME `o2501o9.mesh-registry.fly.dev` and
  `_fly-ownership.api.packages` TXT `app-o2501o9` — same.
- `*` ALIAS `cname.vercel-dns-016.com` — a Vercel wildcard that answers
  `DEPLOYMENT_NOT_FOUND` for every name, including ones that look real like
  `docs.` and `blog.`. Recreating it would shadow future subdomains with a 404.

## Steps

The registry cannot start until steps 1 and 2 are done: it reads its whole
configuration from the environment and exits on the first missing variable.
Steps 3–5 are the DNS cutover and can wait.

### 1. R2 credentials for the registry

The container reaches R2 over its S3 API, so it needs a key pair rather than a
binding.

1. <https://dash.cloudflare.com/?to=/:account/r2/api-tokens> → **Create API
   token**.
2. Permission **Object Read & Write**, scoped to the `mesh-packages` bucket.
3. Copy the **Access Key ID** and **Secret Access Key** — the secret is shown
   once.
4. Store them, pasting each value at the prompt:

       cd registry
       npx wrangler secret put STORAGE_ACCESS_KEY_ID
       npx wrangler secret put STORAGE_SECRET_ACCESS_KEY

### 2. GitHub OAuth app for publisher sign-in

1. <https://github.com/settings/developers> → **New OAuth App**.
2. Homepage URL `https://packages.meshlang.dev`, Authorization callback URL
   `https://api.packages.meshlang.dev/auth/github/callback`. The callback must
   match `GITHUB_CALLBACK_URL` in `registry/wrangler.jsonc` exactly.
3. Generate a client secret, then:

       cd registry
       npx wrangler secret put GITHUB_CLIENT_SECRET

4. Put the **Client ID** — not a secret — into `GITHUB_CLIENT_ID` under `vars`
   in `registry/wrangler.jsonc`, and redeploy.

### 3. Deploy credentials for CI

`CLOUDFLARE_ACCOUNT_ID` is already set. The token is not:

1. <https://dash.cloudflare.com/profile/api-tokens> → **Create Token** →
   **Custom token**, with Account permissions **Workers Scripts: Edit**,
   **Workers R2 Storage: Edit** and **Cloudflare Containers: Edit**.
2. `gh secret set CLOUDFLARE_API_TOKEN --repo snowdamiz/mesh-lang`

### 4. Move the zone

1. <https://dash.cloudflare.com/> → **Add a site** → `meshlang.dev`, free plan.
   Cloudflare scans public DNS and imports what it finds; check the result
   against the table above. Scans routinely miss the TXT and CAA records, and
   the GitHub Pages ones must be **DNS only** (grey cloud).
2. Cloudflare shows two assigned nameservers. `meshlang.dev` is registered at
   Vercel, so change them there:
   <https://vercel.com/120356aas-projects/~/domains> → `meshlang.dev` →
   nameservers → use Cloudflare's pair. Propagation is usually minutes.
3. Confirm the docs site survived — this is the one irreversible-feeling step,
   and it is what to check first:

       curl -sI https://meshlang.dev | head -1      # expect 200
       dig +short meshlang.dev                      # expect 185.199.*

### 5. Attach the custom domains

Add to `registry/wrangler.jsonc` and `packages-website/wrangler.jsonc`:

    "routes": [{ "pattern": "api.packages.meshlang.dev", "custom_domain": true }]
    "routes": [{ "pattern": "packages.meshlang.dev", "custom_domain": true }]

then redeploy both. These are left out until the zone exists, because a deploy
naming a custom domain on a zone Cloudflare does not hold fails outright.

Once `api.packages.meshlang.dev` resolves, drop the `REGISTRY_URL` override in
`packages-website/wrangler.jsonc` or point it at the real host.

The domain can stay attached to Vercel or be removed there; once the
nameservers move, Vercel's DNS records stop being authoritative either way.
