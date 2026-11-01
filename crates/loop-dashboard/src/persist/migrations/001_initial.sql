-- Dashboard store schema v1.
--
-- One writer connection (spawn_blocking); WAL + synchronous=FULL for
-- durability across process crashes. All timestamps are unix epoch
-- MILLISECONDS (consistent with loop-ai / loop-agent conventions).

CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

-- Completed or in-flight inference traces.
CREATE TABLE IF NOT EXISTS traces (
  trace_id       TEXT PRIMARY KEY,
  root_span_id   TEXT NOT NULL,
  operation      TEXT NOT NULL,           -- e.g. 'dashboard.chat'
  status         TEXT NOT NULL,           -- running|success|error|cancelled_user|cancelled_client|interrupted
  model          TEXT,
  provider       TEXT,
  request_id     TEXT,
  session_id     TEXT,                    -- playground conversation id when known
  started_at_ms  INTEGER NOT NULL,
  ended_at_ms    INTEGER,
  duration_ms    INTEGER,
  input_tokens   INTEGER,
  output_tokens  INTEGER,
  ttft_ms        INTEGER,
  error_message  TEXT,
  params_json    TEXT,                    -- whitelisted inference parameters only
  content_captured INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_traces_started ON traces(started_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_traces_status ON traces(status);
CREATE INDEX IF NOT EXISTS idx_traces_model  ON traces(model);
CREATE INDEX IF NOT EXISTS idx_traces_request ON traces(request_id);

-- Spans of a trace (parent-child via span_id/parent_span_id).
CREATE TABLE IF NOT EXISTS spans (
  trace_id       TEXT NOT NULL REFERENCES traces(trace_id) ON DELETE CASCADE,
  span_id        TEXT NOT NULL,
  parent_span_id TEXT,
  name           TEXT NOT NULL,
  kind           TEXT NOT NULL,           -- phase|llm|stream
  status         TEXT NOT NULL,           -- running|success|error|cancelled_user|cancelled_client|interrupted
  start_ms       INTEGER NOT NULL,
  end_ms         INTEGER,
  duration_ms    INTEGER,
  attributes_json TEXT,
  PRIMARY KEY (trace_id, span_id)
);
CREATE INDEX IF NOT EXISTS idx_spans_trace ON spans(trace_id);

-- Structured events recorded on a trace.
CREATE TABLE IF NOT EXISTS trace_events (
  trace_id  TEXT NOT NULL REFERENCES traces(trace_id) ON DELETE CASCADE,
  seq       INTEGER NOT NULL,
  at_ms     INTEGER NOT NULL,
  name      TEXT NOT NULL,
  data_json TEXT,
  PRIMARY KEY (trace_id, seq)
);

-- Request-oriented view of inference traffic (one row per request).
CREATE TABLE IF NOT EXISTS requests (
  request_id     TEXT PRIMARY KEY,
  trace_id       TEXT NOT NULL,
  conversation_id TEXT,
  model          TEXT,
  provider       TEXT,
  status         TEXT NOT NULL,           -- running|success|error|cancelled_user|cancelled_client|interrupted
  started_at_ms  INTEGER NOT NULL,
  ended_at_ms    INTEGER,
  duration_ms    INTEGER,
  input_tokens   INTEGER,
  output_tokens  INTEGER,
  ttft_ms        INTEGER,
  cancel_reason  TEXT,                    -- user|client_disconnect|timeout|NULL
  error_message  TEXT
);
CREATE INDEX IF NOT EXISTS idx_requests_started ON requests(started_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_requests_status  ON requests(status);
CREATE INDEX IF NOT EXISTS idx_requests_trace   ON requests(trace_id);

-- Playground conversations (user-visible chat history; distinct from traces).
CREATE TABLE IF NOT EXISTS conversations (
  conversation_id TEXT PRIMARY KEY,
  title           TEXT NOT NULL,
  created_at_ms   INTEGER NOT NULL,
  updated_at_ms   INTEGER NOT NULL,
  model           TEXT
);
CREATE INDEX IF NOT EXISTS idx_conversations_updated ON conversations(updated_at_ms DESC);

-- Ordered messages of a conversation.
CREATE TABLE IF NOT EXISTS conversation_messages (
  conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
  message_id      TEXT NOT NULL,
  seq             INTEGER NOT NULL,
  role            TEXT NOT NULL,           -- user|assistant
  content         TEXT NOT NULL,           -- markdown text
  created_at_ms   INTEGER NOT NULL,
  status          TEXT NOT NULL,           -- complete|streaming|error|cancelled
  error_message   TEXT,
  model           TEXT,
  request_id      TEXT,
  trace_id        TEXT,
  PRIMARY KEY (conversation_id, message_id)
);
CREATE INDEX IF NOT EXISTS idx_convmsg_conv_seq ON conversation_messages(conversation_id, seq);

-- Dashboard-owned settings (content capture flag, retention).
CREATE TABLE IF NOT EXISTS dashboard_settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
