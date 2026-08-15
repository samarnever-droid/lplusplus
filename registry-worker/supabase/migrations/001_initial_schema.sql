-- L++ Package Registry Schema
-- Supabase Project: (create new or use existing)
-- Enable RLS on all tables

-- API Keys table (scoped, rate-limited)
CREATE TABLE IF NOT EXISTS api_keys (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  label text NOT NULL,
  key_hash text NOT NULL UNIQUE,
  key_prefix text NOT NULL,
  scopes text[] NOT NULL DEFAULT ARRAY['db:read', 'db:write', 'api:read', 'api:write'],
  rate_limit_per_min integer NOT NULL DEFAULT 60,
  daily_request_quota integer NOT NULL DEFAULT 10000,
  active boolean NOT NULL DEFAULT true,
  expires_at timestamptz,
  last_used_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS api_keys_key_hash_idx ON api_keys(key_hash);
CREATE INDEX IF NOT EXISTS api_keys_prefix_idx ON api_keys(key_prefix);

-- Packages table
CREATE TABLE IF NOT EXISTS packages (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name text NOT NULL,
  version text NOT NULL,
  metadata jsonb NOT NULL DEFAULT '{}',
  published_by uuid REFERENCES api_keys(id),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE(name, version)
);

CREATE INDEX IF NOT EXISTS packages_name_idx ON packages(name);
CREATE INDEX IF NOT EXISTS packages_name_version_idx ON packages(name, version);

-- Request logs for analytics
CREATE TABLE IF NOT EXISTS request_logs (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  api_key_id uuid REFERENCES api_keys(id) ON DELETE SET NULL,
  method text NOT NULL,
  path text NOT NULL,
  status integer NOT NULL,
  duration_ms integer NOT NULL DEFAULT 0,
  ip text,
  user_agent text,
  error_code text,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS request_logs_key_time_idx ON request_logs(api_key_id, created_at DESC);
CREATE INDEX IF NOT EXISTS request_logs_time_idx ON request_logs(created_at DESC);

-- RPC function to get full manifest
CREATE OR REPLACE FUNCTION get_registry_manifest()
RETURNS jsonb
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public
AS $$
DECLARE
  result jsonb;
BEGIN
  SELECT json_build_object(
    'registry', json_build_object(
      'name', 'L++ Package Registry',
      'version', '1.0.0',
      'url', current_setting('app.registry_url', true),
      'description', 'Official L++ package registry — powered by Supabase + Cloudflare'
    ),
    'packages', COALESCE(
      (SELECT json_object_agg(p.name, p.metadata)
       FROM packages p
       GROUP BY p.name),
      '{}'::jsonb
    )
  ) INTO result;
  RETURN result;
END;
$$;

-- Trigger to update updated_at
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
  NEW.updated_at = now();
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER update_packages_updated_at
  BEFORE UPDATE ON packages
  FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- RLS policies
ALTER TABLE api_keys ENABLE ROW LEVEL SECURITY;
ALTER TABLE packages ENABLE ROW LEVEL SECURITY;
ALTER TABLE request_logs ENABLE ROW LEVEL SECURITY;

-- Public read access to packages
CREATE POLICY "public_read_packages" ON packages FOR SELECT TO anon, authenticated USING (true);

-- Service role can do everything (bypass RLS via SECURITY DEFINER function)
CREATE POLICY "service_full_access" ON api_keys FOR ALL TO service_role USING (true) WITH CHECK (true);
CREATE POLICY "service_full_packages" ON packages FOR ALL TO service_role USING (true) WITH CHECK (true);
CREATE POLICY "service_full_logs" ON request_logs FOR ALL TO service_role USING (true) WITH CHECK (true);

-- Insert default API key for development (sk_layer_devkey123)
INSERT INTO api_keys (label, key_hash, key_prefix, scopes, rate_limit_per_min, daily_request_quota)
VALUES (
  'Development Key',
  encode(digest('sk_layer_devkey123', 'sha256'), 'hex'),
  'sk_layer_d',
  ARRAY['db:read', 'db:write', 'api:read', 'api:write'],
  60,
  1000
) ON CONFLICT (key_hash) DO NOTHING;
