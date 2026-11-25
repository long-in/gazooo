-- docs/03-data-model.md v1

-- 生成ジョブ(1 リクエスト = 1 ジョブ、N 枚の画像を持つ)
CREATE TABLE jobs (
  id            TEXT PRIMARY KEY,
  status        TEXT NOT NULL CHECK (status IN
                  ('queued','running','completed','failed','cancelled')),
  position      INTEGER NOT NULL,
  engine        TEXT NOT NULL,
  params_json   TEXT NOT NULL,
  prompt_text   TEXT NOT NULL,
  model_name    TEXT NOT NULL,
  error_code    TEXT,
  error_message TEXT,
  remote_id     TEXT,
  created_at    INTEGER NOT NULL,
  started_at    INTEGER,
  finished_at   INTEGER
);
CREATE INDEX idx_jobs_status_position ON jobs(status, position);
CREATE INDEX idx_jobs_created_at ON jobs(created_at DESC);

-- 生成画像
CREATE TABLE images (
  id            TEXT PRIMARY KEY,
  job_id        TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
  file_path     TEXT NOT NULL,
  thumb_path    TEXT NOT NULL,
  width         INTEGER NOT NULL,
  height        INTEGER NOT NULL,
  seed          INTEGER NOT NULL,
  file_size     INTEGER NOT NULL,
  is_favorite   INTEGER NOT NULL DEFAULT 0,
  created_at    INTEGER NOT NULL
);
CREATE INDEX idx_images_created_at ON images(created_at DESC);
CREATE INDEX idx_images_favorite ON images(is_favorite, created_at DESC);
CREATE INDEX idx_images_job ON images(job_id);

-- プリセット
CREATE TABLE presets (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL UNIQUE,
  params_json TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);

-- 設定(キー・バリュー)
CREATE TABLE settings (
  key        TEXT PRIMARY KEY,
  value_json TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
