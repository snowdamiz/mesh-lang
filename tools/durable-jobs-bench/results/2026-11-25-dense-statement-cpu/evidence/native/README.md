# Dense statement CPU native capture

The PostgreSQL 17.11 extension SETUP released exit 0 at 02:31:23 UTC.
The Linux smoke passed preload, backend exit, and off-clock publish in
both modes. The enabled mode also passed nested SPI, error, abort,
statistics reset, and client idle controls.
[SETUP release](setup/controller-release.json) · [Setup receipt](setup/receipt.json)

Both reversed groups captured all 24 warmup and measured arms. Each raw
arm records exit 0. The scorer left 3,761 statements unmapped in its
first attribution check and TIMING released exit 1 at 02:35:03 UTC.
This archive retains the original scorer failure and both groups for
offline reconciliation.
[TIMING release](timing/controller-release.json) · [Controller traceback](timing/controller.log)
