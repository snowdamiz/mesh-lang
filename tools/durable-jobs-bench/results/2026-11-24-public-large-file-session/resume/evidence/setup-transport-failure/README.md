# File-session SETUP transport failure

The original grant ran from 23:30:35 to 23:55:35 UTC on November 24.
The local client hit its 1,200-second timeout after the workstation
resumed from sleep. The export and release probe timed out at TCP/22.
The release remains pending. The original export is zero bytes.

The setup transcript shows the stage and fixed pin checks, followed
by GNU sha256sum's warning about seven malformed member records.
It contains no completed runtime selection or proof export.

The [original records](raw-original.tar.gz) include the unchanged
transcripts, zero-byte export, pending release, and sequential readback
failure. See the [summary](summary.json). The existing remote output
needs authenticated readback and the separate stock member postflight
before opening TIMING.

```bash
tar -xOzf raw-original.tar.gz setup/setup.log
tar -xOzf raw-original.tar.gz setup-release-pending.json
```
