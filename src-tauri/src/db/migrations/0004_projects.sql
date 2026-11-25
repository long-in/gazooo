-- プロジェクト(作業フォルダ。docs/40 §3)。画像・ジョブはプロジェクトが消えると未分類へ戻る。
CREATE TABLE projects (
  id               TEXT PRIMARY KEY,
  name             TEXT NOT NULL UNIQUE,
  color            TEXT,
  sort_order       INTEGER NOT NULL,
  last_params_json TEXT,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL
);

ALTER TABLE jobs   ADD COLUMN project_id TEXT REFERENCES projects(id) ON DELETE SET NULL;
ALTER TABLE images ADD COLUMN project_id TEXT REFERENCES projects(id) ON DELETE SET NULL;

CREATE INDEX idx_images_project ON images(project_id, created_at DESC, id DESC);
CREATE INDEX idx_jobs_project   ON jobs(project_id);
