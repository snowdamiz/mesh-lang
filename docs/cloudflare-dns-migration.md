# Moving meshlang.dev onto Cloudflare DNS

The services moved from Fly.io to Cloudflare. Workers custom domains only work
when the zone is on Cloudflare, so `meshlang.dev` has to move off Vercel DNS
(`ns1/ns2.vercel-dns.com`) for `packages.meshlang.dev` and
`api.packages.meshlang.dev` to resolve to the new Workers.

Until that happens the services are reachable at their `workers.dev` URLs and
`meshlang.dev` itself is untouched.

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

1. Add `meshlang.dev` in the Cloudflare dashboard (**Add a site**). Cloudflare
   scans public DNS and imports what it finds; check the imported set against
   the table above, especially the TXT and CAA records, which scans miss.
2. Change the nameservers at the registrar from Vercel's to the pair Cloudflare
   shows. Propagation is usually minutes.
3. Confirm the docs site still serves: `curl -I https://meshlang.dev` should be
   `200` from a `185.199.*` address.
4. Attach the custom domains:

       cd registry && npx wrangler deploy          # api.packages.meshlang.dev
       cd packages-website && npx wrangler deploy  # packages.meshlang.dev

   after adding a `routes` entry with `custom_domain: true` to each
   `wrangler.jsonc`. They are left out until the zone exists, because a deploy
   naming a custom domain on a zone Cloudflare does not hold fails.
5. Leave the domain attached to Vercel or remove it there; once the nameservers
   move, Vercel's DNS records stop being authoritative either way.
