# Native transport check

This setup diagnostic sent the matrix worker and a synthetic inventory to the authorized Linode. Its unavailable CPU IDs forced the worker to stop before it could call an adapter. SSH returned exit status `1`. The worker retained the invocation and error on the host, and the controller downloaded the sanitized archive at [diagnostic-mesh/raw.tar.gz](diagnostic-mesh/raw.tar.gz). This run contributes zero benchmark samples.

The host series and invocation IDs are in [purpose.json](purpose.json) and [request.json](diagnostic-mesh/request.json).
