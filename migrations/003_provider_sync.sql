CREATE TABLE provider_cursors (
 platform TEXT NOT NULL, account_id TEXT NOT NULL, cursor TEXT NOT NULL,
 updated_at INTEGER NOT NULL, PRIMARY KEY(platform, account_id)
);
-- 新建后复制再替换；outbox 没有入向外键，保留对 customers 的复合外键。
CREATE TABLE outbox_v3 (
 send_id TEXT PRIMARY KEY, platform TEXT NOT NULL, account_id TEXT NOT NULL, customer_id TEXT,
 idempotency_key TEXT NOT NULL, recipient TEXT NOT NULL, message TEXT NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('attempting','triggered','unknown','rejected')),
 result TEXT CHECK(result IS NULL OR json_valid(result)), created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
 UNIQUE(platform, account_id, idempotency_key),
 FOREIGN KEY(platform, account_id, customer_id) REFERENCES customers(platform, account_id, customer_id)
);
INSERT INTO outbox_v3 SELECT * FROM outbox;
DROP TABLE outbox;
ALTER TABLE outbox_v3 RENAME TO outbox;
CREATE INDEX provider_inbound ON events(platform, account_id, occurred_at)
 WHERE source='wecom_kf_api' AND verified=1 AND json_extract(payload,'$.direction')='inbound';
PRAGMA user_version = 3;
