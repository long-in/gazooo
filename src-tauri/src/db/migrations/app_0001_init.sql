CREATE TABLE IF NOT EXISTS users (
  id             TEXT PRIMARY KEY,
  name           TEXT NOT NULL,
  color          TEXT NOT NULL,
  auth_kind      TEXT NOT NULL DEFAULT 'none' CHECK (auth_kind IN ('none','pin','passphrase')),
  auth_hash      TEXT,
  pin_length     INTEGER,
  failed_count   INTEGER NOT NULL DEFAULT 0,
  locked_until   INTEGER,
  created_at     INTEGER NOT NULL,
  last_login_at  INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_name ON users(lower(name));

CREATE TABLE IF NOT EXISTS device_settings (
  key        TEXT PRIMARY KEY,
  value_json TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
