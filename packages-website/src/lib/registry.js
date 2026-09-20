import { env } from '$env/dynamic/private';

// The registry's base URL. It is a binding rather than a constant so the site
// can be pointed at the registry's workers.dev URL while
// api.packages.meshlang.dev is still being moved onto Cloudflare.
export const REGISTRY_URL = env.REGISTRY_URL ?? 'https://api.packages.meshlang.dev';
