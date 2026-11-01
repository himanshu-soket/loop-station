-- v2: analytics-friendly token/latency columns already exist in v1; this
-- migration adds the retention bookkeeping table.

CREATE TABLE IF NOT EXISTS retention_runs (
  ran_at_ms   INTEGER PRIMARY KEY,
  deleted_traces  INTEGER NOT NULL,
  deleted_requests INTEGER NOT NULL
);
