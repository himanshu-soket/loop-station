-- v3: record the owning process of each trace.
--
-- `DashboardStore::open` runs crash recovery, but the store is shared by
-- many live processes (the dashboard plus every CLI session). A `running`
-- row may simply belong to a concurrent session that is still working, so
-- recovery must only claim rows whose owner process has exited. `owner_pid`
-- is that owner; NULL means "unknown owner" (legacy rows), which recovery
-- treats as abandoned, preserving the pre-v3 behaviour.

ALTER TABLE traces ADD COLUMN owner_pid INTEGER;
