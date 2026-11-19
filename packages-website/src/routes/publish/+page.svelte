<script>
  import CopyCommand from '$lib/CopyCommand.svelte';
  import { AUTH_URL } from '$lib/site.js';
</script>

<svelte:head>
  <title>Publish a package · Mesh Packages</title>
  <meta name="description" content="Publish a Mesh package: sign in with GitHub, save the token with meshpkg login, then run meshpkg publish." />
  <meta property="og:title" content="Publish a package · Mesh Packages" />
  <meta property="og:description" content="Sign in with GitHub, save the token with meshpkg login, then run meshpkg publish." />
  <meta property="og:url" content="https://packages.meshlang.dev/publish" />
  <link rel="canonical" href="https://packages.meshlang.dev/publish" />
</svelte:head>

<div class="page">
  <header class="page-head">
    <h1 class="app-title">Publish a package</h1>
    <p class="lead">Sign in with GitHub once, then publish from your package directory.</p>
  </header>

  <ol class="steps">
    <li>
      <h2>Sign in with GitHub</h2>
      <p>Each sign-in creates a new publish token and shows it once on the next page.</p>
      <div class="step-body"><a class="btn btn-ink" href={AUTH_URL}>Sign in with GitHub</a></div>
    </li>
    <li>
      <h2>Save the token</h2>
      <p>Paste the token when <code class="ic">meshpkg login</code> asks for it. It is stored in <code class="ic">~/.mesh/credentials</code>.</p>
      <div class="step-body"><CopyCommand text="meshpkg login" label="Copy meshpkg login" /></div>
    </li>
    <li>
      <h2>Publish</h2>
      <p>Run it next to <code class="ic">mesh.toml</code>. The name must start with your GitHub login.</p>
      <div class="step-body">
        <figure class="codeblock">
          <pre><code><span class="pr"># mesh.toml</span>
[package]
name = "your-login/your-package"
version = "1.0.0"
description = "What your package does"
license = "MIT"</code></pre>
        </figure>
        <CopyCommand text="meshpkg publish" label="Copy meshpkg publish" />
      </div>
    </li>
  </ol>

  <section class="rules" aria-labelledby="rules-heading">
    <h2 id="rules-heading">Rules</h2>
    <ul>
      <li>Your GitHub login must match the owner scope: <code class="ic">your-login/…</code>.</li>
      <li>Published versions are immutable. Ship a fix as a new version.</li>
      <li>A package upload can be at most 50 MiB.</li>
    </ul>
  </section>
</div>

<style>
  .steps {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    border-top: 1px solid var(--line);
    counter-reset: step;
  }
  .steps > li {
    display: grid;
    grid-template-columns: 28px minmax(0, 1fr);
    gap: 4px 16px;
    padding: 24px 0;
    border-bottom: 1px solid var(--line);
    counter-increment: step;
  }
  .steps > li::before {
    content: counter(step);
    grid-row: 1 / span 3;
    width: 24px;
    height: 24px;
    margin-top: 1px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: var(--tint);
    font-size: 12.5px;
    font-weight: 500;
    color: var(--ink-2);
    font-variant-numeric: tabular-nums;
  }
  .steps h2 {
    font-size: 16px;
    font-weight: 600;
    letter-spacing: -0.015em;
  }
  .steps p {
    font-size: var(--fs-sm);
    color: var(--mute);
  }
  .steps .step-body {
    margin-top: 12px;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 10px;
    justify-items: start;
  }
  .steps .step-body > :global(.cmd),
  .steps .step-body > .codeblock {
    justify-self: stretch;
    margin: 0;
  }
  .rules {
    margin-top: 56px;
    display: grid;
    gap: 14px;
  }
  .rules h2 {
    font-size: 17px;
    font-weight: 600;
    letter-spacing: -0.015em;
  }
  .rules ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 10px;
  }
  .rules li {
    position: relative;
    padding-left: 20px;
    font-size: 15px;
    color: var(--ink-2);
  }
  .rules li::before {
    content: '';
    position: absolute;
    left: 4px;
    top: 0.7em;
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--faint);
  }
</style>
