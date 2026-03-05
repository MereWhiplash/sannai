-- Sannai Cloud Schema: Initial Migration

-- Organizations
CREATE TABLE organizations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    slug TEXT UNIQUE NOT NULL,
    sso_provider TEXT,
    sso_config JSONB,
    settings JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT now()
);

-- Users
CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id UUID REFERENCES organizations(id),
    email TEXT NOT NULL,
    name TEXT,
    role TEXT DEFAULT 'member',
    created_at TIMESTAMPTZ DEFAULT now(),
    last_active_at TIMESTAMPTZ,
    UNIQUE(org_id, email)
);

-- Sessions (synced from local agents)
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    user_id UUID REFERENCES users(id),
    tool TEXT NOT NULL,
    project_name TEXT,
    prompt_count INTEGER DEFAULT 0,
    duration_seconds INTEGER,
    started_at TIMESTAMPTZ NOT NULL,
    ended_at TIMESTAMPTZ,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT now()
);

-- Session events
CREATE TABLE session_events (
    id BIGSERIAL PRIMARY KEY,
    session_id TEXT REFERENCES sessions(id),
    event_type TEXT NOT NULL,
    content TEXT,
    context_files JSONB,
    timestamp TIMESTAMPTZ NOT NULL,
    metadata JSONB DEFAULT '{}'
);

-- Commit links
CREATE TABLE commit_links (
    id BIGSERIAL PRIMARY KEY,
    session_id TEXT REFERENCES sessions(id),
    commit_sha TEXT NOT NULL,
    repo_full_name TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT now(),
    UNIQUE(session_id, commit_sha)
);

-- PR links
CREATE TABLE pr_sessions (
    id BIGSERIAL PRIMARY KEY,
    pr_number INTEGER NOT NULL,
    repo_full_name TEXT NOT NULL,
    session_id TEXT REFERENCES sessions(id),
    created_at TIMESTAMPTZ DEFAULT now(),
    UNIQUE(pr_number, repo_full_name, session_id)
);

-- Indexes
CREATE INDEX idx_sessions_user ON sessions(user_id);
CREATE INDEX idx_sessions_started ON sessions(started_at);
CREATE INDEX idx_events_session ON session_events(session_id);
CREATE INDEX idx_commits_sha ON commit_links(commit_sha);
CREATE INDEX idx_commits_repo ON commit_links(repo_full_name);
CREATE INDEX idx_pr_repo ON pr_sessions(repo_full_name, pr_number);
