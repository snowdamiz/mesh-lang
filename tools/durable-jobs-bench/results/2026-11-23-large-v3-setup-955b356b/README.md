# Historical strict LARGE SETUP

The controller released the original `large-v3-955b356b` SETUP with
exit 0 at 02:11:45 UTC. The exact candidate used
`locked_prepare_pg17_v1`. The diagnostic ledger reconciled all 64 jobs
with one attempt and one durable effect each. It recorded a peak of
65 database connections and 16 executing jobs.
[Release](controller-release.json) · [Smoke archive](raw.tar.gz)

The build used the pinned stock compiler and exact candidate runtime
source. The resulting ELF SHA-256 starts with `bb94ad47b161`.
The [artifact binding](artifact.json) matches the digest in the smoke
ledger. The [timing inventory](timing-inventory.json) binds that build
to the separate historical comparison with the frozen stock, River,
and Oban lanes.
