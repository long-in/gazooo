-- docs/03-data-model.md「マイグレーション 0002」
-- 画像を持つ終了済みジョブをキュー画面から隠す(ジョブを消すと画像の記録が CASCADE で消えるため)
ALTER TABLE jobs ADD COLUMN queue_hidden INTEGER NOT NULL DEFAULT 0;
