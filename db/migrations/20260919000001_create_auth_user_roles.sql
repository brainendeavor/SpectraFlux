-- migrate:up
CREATE TABLE IF NOT EXISTS auth_user_roles (
    user_id VARCHAR(255) NOT NULL,
    tenant_id VARCHAR(255) NOT NULL DEFAULT '',
    roles JSONB NOT NULL DEFAULT '[]'::jsonb,
    permissions JSONB NOT NULL DEFAULT '[]'::jsonb,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, tenant_id)
);

CREATE INDEX IF NOT EXISTS idx_auth_user_roles_lookup 
    ON auth_user_roles (user_id, tenant_id);

CREATE INDEX IF NOT EXISTS idx_auth_user_roles_roles 
    ON auth_user_roles USING GIN (roles);

-- migrate:down
DROP INDEX IF EXISTS idx_auth_user_roles_roles;
DROP INDEX IF EXISTS idx_auth_user_roles_lookup;
DROP TABLE IF EXISTS auth_user_roles;
