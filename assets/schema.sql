CREATE TABLE runs (
    id TEXT PRIMARY KEY, model TEXT NOT NULL, started REAL NOT NULL,
    first_seen REAL NOT NULL, last_seen REAL NOT NULL,
    requests INTEGER NOT NULL, prompt_tokens INTEGER NOT NULL,
    reused INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
    prompt_ms REAL NOT NULL, decode_ms REAL NOT NULL,
    drafts_offered INTEGER NOT NULL, drafts_accepted INTEGER NOT NULL
);
CREATE TABLE requests (
    id TEXT PRIMARY KEY, run_id TEXT NOT NULL, started REAL NOT NULL,
    day TEXT NOT NULL, model TEXT NOT NULL, duration_s REAL NOT NULL,
    prompt_tokens INTEGER NOT NULL, reused INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL, prompt_ms REAL NOT NULL,
    decode_ms REAL NOT NULL, finish TEXT NOT NULL
);
CREATE INDEX requests_day ON requests(day);
CREATE TABLE log_imports (
    id TEXT PRIMARY KEY, model TEXT NOT NULL, date_from TEXT,
    date_to TEXT NOT NULL, day TEXT, source_path TEXT NOT NULL,
    source_sha256 TEXT NOT NULL, line_start INTEGER NOT NULL,
    line_end INTEGER NOT NULL, requests INTEGER NOT NULL,
    prompt_tokens INTEGER NOT NULL, reused INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL, prompt_ms REAL NOT NULL,
    decode_ms REAL NOT NULL, drafts_offered INTEGER NOT NULL,
    drafts_accepted INTEGER NOT NULL, imported_at REAL NOT NULL
);
