CREATE TABLE customers (
 platform TEXT NOT NULL, account_id TEXT NOT NULL, customer_id TEXT NOT NULL,
 display_name TEXT NOT NULL, recipient TEXT, external_id TEXT,
 status TEXT NOT NULL CHECK(status IN ('lead','active','dormant','closed')),
 tags TEXT NOT NULL CHECK(json_valid(tags)), notes TEXT NOT NULL,
 version INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
 PRIMARY KEY(platform, account_id, customer_id),
 UNIQUE(platform, account_id, external_id)
);
CREATE TABLE events (
 event_id TEXT PRIMARY KEY, platform TEXT NOT NULL, account_id TEXT NOT NULL, customer_id TEXT,
 event_key TEXT, kind TEXT NOT NULL, source TEXT NOT NULL,
 verified INTEGER NOT NULL CHECK(verified IN (0,1)), occurred_at INTEGER NOT NULL,
 observed_at INTEGER NOT NULL, payload TEXT NOT NULL CHECK(json_valid(payload)),
 state TEXT NOT NULL CHECK(state IN ('recorded','pending','leased','acknowledged')),
 consumer TEXT, lease_token TEXT, lease_until INTEGER,
 UNIQUE(platform, account_id, event_key),
 FOREIGN KEY(platform, account_id, customer_id) REFERENCES customers(platform, account_id, customer_id)
);
CREATE INDEX inbox_pending ON events(platform, account_id, state, observed_at, event_id);
CREATE INDEX customer_timeline ON events(platform, account_id, customer_id, observed_at, event_id);
CREATE TABLE followups (
 task_id TEXT PRIMARY KEY, platform TEXT NOT NULL, account_id TEXT NOT NULL, customer_id TEXT NOT NULL,
 title TEXT NOT NULL, due_at INTEGER NOT NULL, owner TEXT NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('pending','done','cancelled')),
 outcome TEXT, created_at INTEGER NOT NULL, completed_at INTEGER,
 FOREIGN KEY(platform, account_id, customer_id) REFERENCES customers(platform, account_id, customer_id)
);
CREATE INDEX followup_due ON followups(platform, account_id, status, due_at, task_id);
CREATE TABLE analyses (
 analysis_id TEXT PRIMARY KEY, platform TEXT NOT NULL, account_id TEXT NOT NULL, customer_id TEXT NOT NULL,
 summary TEXT NOT NULL, created_at INTEGER NOT NULL,
 FOREIGN KEY(platform, account_id, customer_id) REFERENCES customers(platform, account_id, customer_id)
);
CREATE TABLE analysis_evidence (
 analysis_id TEXT NOT NULL REFERENCES analyses(analysis_id), event_id TEXT NOT NULL REFERENCES events(event_id),
 PRIMARY KEY(analysis_id, event_id)
);
CREATE TABLE outbox (
 send_id TEXT PRIMARY KEY, platform TEXT NOT NULL, account_id TEXT NOT NULL, customer_id TEXT,
 idempotency_key TEXT NOT NULL, recipient TEXT NOT NULL, message TEXT NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('attempting','triggered','unknown')),
 result TEXT CHECK(result IS NULL OR json_valid(result)), created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
 UNIQUE(platform, account_id, idempotency_key),
 FOREIGN KEY(platform, account_id, customer_id) REFERENCES customers(platform, account_id, customer_id)
);
CREATE TABLE listener_snapshots (
 platform TEXT NOT NULL, account_id TEXT NOT NULL, snapshot TEXT NOT NULL CHECK(json_valid(snapshot)),
 updated_at INTEGER NOT NULL, PRIMARY KEY(platform, account_id)
);
CREATE TABLE operation_receipts (
 platform TEXT NOT NULL, account_id TEXT NOT NULL, operation TEXT NOT NULL, idempotency_key TEXT NOT NULL,
 request TEXT NOT NULL CHECK(json_valid(request)), result TEXT NOT NULL CHECK(json_valid(result)), created_at INTEGER NOT NULL,
 PRIMARY KEY(platform,account_id,operation,idempotency_key)
);
PRAGMA user_version = 1;
