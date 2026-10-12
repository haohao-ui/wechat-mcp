CREATE TABLE listener_settings (
 platform TEXT NOT NULL, account_id TEXT NOT NULL,
 enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
 interval_seconds INTEGER NOT NULL CHECK(interval_seconds BETWEEN 2 AND 60),
 updated_at INTEGER NOT NULL,
 PRIMARY KEY(platform,account_id)
);
PRAGMA user_version = 2;
