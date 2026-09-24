+++
title = "state.db, blobs and index.db"
level = "container"
parent = "kitsu"
technology = "SQLite (WAL), content-addressed files"
+++
Execution state and caches under the workspace directory. `state.db` is
written only through the store; `index.db` can be deleted and rebuilt.
