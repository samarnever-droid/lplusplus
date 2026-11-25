-- L++ Ecosystem PostgreSQL 17 Schema (for blitz.cloud API service)

CREATE TABLE IF NOT EXISTS package_stars (
  package_name TEXT NOT NULL,
  clerk_user_id TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (package_name, clerk_user_id)
);

CREATE TABLE IF NOT EXISTS package_downloads (
  package_name TEXT PRIMARY KEY,
  downloads BIGINT NOT NULL DEFAULT 0,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS playground_snippets (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL DEFAULT 'Untitled Snippet',
  code TEXT NOT NULL,
  clerk_user_id TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_snippets_user ON playground_snippets (clerk_user_id);
