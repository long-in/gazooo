-- AI サポートの会話履歴(docs/03-data-model.md「マイグレーション 0003」)
CREATE TABLE assistant_conversations (
  id         TEXT PRIMARY KEY,
  title      TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX idx_assistant_conv_updated ON assistant_conversations(updated_at DESC);

CREATE TABLE assistant_messages (
  id               TEXT PRIMARY KEY,
  conversation_id  TEXT NOT NULL REFERENCES assistant_conversations(id) ON DELETE CASCADE,
  role             TEXT NOT NULL CHECK (role IN ('user','assistant')),
  text             TEXT NOT NULL,
  params_json      TEXT,
  suggestions_json TEXT,
  created_at       INTEGER NOT NULL
);
CREATE INDEX idx_assistant_msg_conv ON assistant_messages(conversation_id, created_at);
