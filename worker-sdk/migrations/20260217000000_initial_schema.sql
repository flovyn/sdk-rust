-- Initial schema for local agent storage (SQLite)
--
-- Four tables: checkpoints, entries, tasks, signals
-- Matches the schema defined in the async-agent-support design doc.

CREATE TABLE IF NOT EXISTS checkpoints (
    agent_id      TEXT NOT NULL,
    segment       INTEGER NOT NULL,
    state         TEXT NOT NULL,
    leaf_entry_id TEXT,
    token_usage   TEXT,
    created_at    TEXT NOT NULL,
    PRIMARY KEY (agent_id, segment)
);

CREATE TABLE IF NOT EXISTS entries (
    entry_id   TEXT PRIMARY KEY,
    agent_id   TEXT NOT NULL,
    parent_id  TEXT,
    segment    INTEGER NOT NULL,
    role       TEXT NOT NULL,
    content    TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tasks (
    task_id      TEXT PRIMARY KEY,
    agent_id     TEXT NOT NULL,
    kind         TEXT NOT NULL,
    input        TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'pending',
    output       TEXT,
    error        TEXT,
    created_at   TEXT NOT NULL,
    completed_at TEXT
);

CREATE TABLE IF NOT EXISTS signals (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id    TEXT NOT NULL,
    signal_name TEXT NOT NULL,
    payload     TEXT NOT NULL,
    created_at  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_entries_agent ON entries(agent_id, segment);
CREATE INDEX IF NOT EXISTS idx_tasks_agent ON tasks(agent_id, status);
CREATE INDEX IF NOT EXISTS idx_signals_agent ON signals(agent_id, signal_name);
