+++
title = "Stop retrying forever when the upstream times out"
scope = ["payments.py"]
checks = ["retries"]
+++
Last Tuesday the upstream browned out for 20 minutes and every web worker
sat in `charge()` spinning on timeouts. That made our outage longer than
theirs.
