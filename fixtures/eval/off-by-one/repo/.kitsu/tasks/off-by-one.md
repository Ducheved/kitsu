+++
title = "The admin user list hides rows"
scope = ["pager.py"]
checks = ["pager"]
+++
With 10 users and 3 per page the admin list says there are 3 pages, and
page 1 starts at the fourth user. `test_pager` reproduces it.
