# Confirmed-grant overlap SETUP recovery

The original Linode SETUP finished inside its grant with `native.exit=0`
and `SETUP-PASS`. The interrupted client left its original launch log and
grant. A fenced, read-only export recovered the complete native archive.

The first local replay found a missing `main.mpl` beside the campaign
script. The controller supplied the exact pinned logger-off fixture
`97b8e75f8568…` already in `overlay/fixture`. The sealed verifier then
replayed the semantic cases and both 1,024-job grant-age diagnostics.
The controller issued a separate positive release for the original
native completion.
[Recovered release](controller-release.json) · [Raw proof](raw.tar.gz) ·
[Verified setup](verified.json) · [First replay release](controller-release-before-local-fixture-binding.json)
