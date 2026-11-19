CREATE TABLE IF NOT EXISTS people (
    id text PRIMARY KEY, name text NOT NULL, email text UNIQUE, password text,
    subject text UNIQUE, kind text NOT NULL DEFAULT 'user', owner boolean NOT NULL DEFAULT false
);
CREATE TABLE IF NOT EXISTS tokens (
    digest text PRIMARY KEY, person text NOT NULL REFERENCES people, label text NOT NULL,
    expires timestamptz, created timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS invites (digest text PRIMARY KEY, email text NOT NULL, expires timestamptz NOT NULL);
CREATE TABLE IF NOT EXISTS groups (id text PRIMARY KEY, name text UNIQUE NOT NULL);
CREATE TABLE IF NOT EXISTS group_members (group_id text REFERENCES groups, person text REFERENCES people, PRIMARY KEY(group_id,person));
CREATE TABLE IF NOT EXISTS topics (slug text PRIMARY KEY, name text NOT NULL, seq integer NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS missions (
    id text PRIMARY KEY, topic text NOT NULL REFERENCES topics, title text NOT NULL, description text NOT NULL DEFAULT '',
    created timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS mission_statuses (
    id text PRIMARY KEY, name text NOT NULL UNIQUE CHECK (btrim(name) <> ''),
    color text NOT NULL, position integer NOT NULL
);
INSERT INTO mission_statuses(id,name,color,position)
    SELECT initial.* FROM (VALUES ('open','Open','slate',0),('in-progress','In progress','amber',1),('done','Done','violet',2))
    AS initial(id,name,color,position) WHERE NOT EXISTS (SELECT 1 FROM mission_statuses);
ALTER TABLE missions ADD COLUMN IF NOT EXISTS status text NOT NULL DEFAULT 'open' REFERENCES mission_statuses(id);
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema=current_schema() AND table_name='missions' AND column_name='archived') THEN
        UPDATE missions SET status='done' WHERE archived;
        ALTER TABLE missions DROP COLUMN archived;
    END IF;
END $$;
ALTER TABLE people ADD COLUMN IF NOT EXISTS admin boolean NOT NULL DEFAULT false;
ALTER TABLE invites ADD COLUMN IF NOT EXISTS admin boolean NOT NULL DEFAULT false;
CREATE TABLE IF NOT EXISTS members (mission text REFERENCES missions, principal text NOT NULL, PRIMARY KEY(mission,principal));
CREATE TABLE IF NOT EXISTS repos (mission text REFERENCES missions, name text NOT NULL, remote text NOT NULL, branch text NOT NULL,
    recovery text, status jsonb NOT NULL DEFAULT '{}', PRIMARY KEY(mission,name));
CREATE TABLE IF NOT EXISTS threads (
    id text PRIMARY KEY, mission text NOT NULL REFERENCES missions, title text NOT NULL, kind text NOT NULL,
    external_key text, created timestamptz DEFAULT now(), UNIQUE(mission,external_key)
);
CREATE UNIQUE INDEX IF NOT EXISTS hosted_titles ON threads(mission,lower(title)) WHERE kind IN ('main','hosted');
CREATE TABLE IF NOT EXISTS agents (
    id text PRIMARY KEY, name text UNIQUE NOT NULL, instructions text NOT NULL, model text NOT NULL, creator text REFERENCES people
);
CREATE TABLE IF NOT EXISTS participants (thread text REFERENCES threads, principal text NOT NULL, PRIMARY KEY(thread,principal));
CREATE TABLE IF NOT EXISTS skills (
    id text PRIMARY KEY, name text UNIQUE NOT NULL, body text NOT NULL, remote text, ref text, path text,
    revision text, creator text REFERENCES people, sync_error text
);
CREATE TABLE IF NOT EXISTS attachments (skill text REFERENCES skills, target text NOT NULL, PRIMARY KEY(skill,target));
CREATE TABLE IF NOT EXISTS messages (
    id bigserial PRIMARY KEY, thread text NOT NULL REFERENCES threads, author text NOT NULL, source text NOT NULL, body text NOT NULL,
    origin_key text, commits jsonb NOT NULL DEFAULT '{}', actor text NOT NULL, created timestamptz NOT NULL DEFAULT now(),
    embedding real[], UNIQUE(thread, origin_key)
);
CREATE INDEX IF NOT EXISTS message_text ON messages USING gin(to_tsvector('english',body));
CREATE INDEX IF NOT EXISTS message_thread ON messages(thread,id);
CREATE TABLE IF NOT EXISTS sessions (
    id text PRIMARY KEY, thread text REFERENCES threads, agent text REFERENCES agents, native text,
    delivered bigint[] NOT NULL DEFAULT '{}', UNIQUE(thread,agent)
);
CREATE TABLE IF NOT EXISTS runs (
    id text PRIMARY KEY, session text REFERENCES sessions, dispatcher text REFERENCES people, trigger bigint REFERENCES messages,
    status text NOT NULL, readers text[] NOT NULL, grants jsonb NOT NULL DEFAULT '{}', token_digest text UNIQUE,
    model text NOT NULL, instructions text NOT NULL, partial text NOT NULL DEFAULT '', error text,
    created timestamptz NOT NULL DEFAULT now(), updated timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX IF NOT EXISTS active_session ON runs(session) WHERE status IN ('queued','working','interrupted','stopping');
CREATE TABLE IF NOT EXISTS run_inputs (
    id bigserial PRIMARY KEY, run text REFERENCES runs, message bigint REFERENCES messages, kind text NOT NULL, sent boolean NOT NULL DEFAULT false
);
CREATE TABLE IF NOT EXISTS questions (
    id text PRIMARY KEY, run text REFERENCES runs, request jsonb NOT NULL, prompt jsonb NOT NULL, answer jsonb, answered_by text REFERENCES people,
    sent boolean NOT NULL DEFAULT false, blocking boolean NOT NULL DEFAULT true
);
CREATE TABLE IF NOT EXISTS subscriptions (person text REFERENCES people, thread text REFERENCES threads, mode text NOT NULL,
    seen bigint NOT NULL DEFAULT 0, PRIMARY KEY(person,thread));
CREATE TABLE IF NOT EXISTS notifications (
    id bigserial PRIMARY KEY, person text REFERENCES people, thread text REFERENCES threads, message bigint REFERENCES messages,
    run text REFERENCES runs, reason text NOT NULL, summary text NOT NULL, seen boolean NOT NULL DEFAULT false, created timestamptz DEFAULT now()
);
CREATE TABLE IF NOT EXISTS devices (endpoint text PRIMARY KEY, person text REFERENCES people, subscription jsonb NOT NULL);
CREATE TABLE IF NOT EXISTS routines (
    id text PRIMARY KEY, name text NOT NULL, owner_id text REFERENCES people, command text NOT NULL, cron text, hook_hash text UNIQUE NOT NULL,
    repos jsonb NOT NULL DEFAULT '[]', enabled boolean NOT NULL DEFAULT true, next_at timestamptz
);
CREATE TABLE IF NOT EXISTS jobs (
    id text PRIMARY KEY, routine text REFERENCES routines, owner_id text REFERENCES people, payload jsonb NOT NULL, status text NOT NULL,
    token_digest text UNIQUE, output text NOT NULL DEFAULT '', created timestamptz DEFAULT now()
);
CREATE TABLE IF NOT EXISTS oidc_flows (
    state text PRIMARY KEY, nonce text NOT NULL, verifier text NOT NULL, expires timestamptz NOT NULL, cli boolean NOT NULL DEFAULT false
);
CREATE OR REPLACE FUNCTION can_read(uid text, mid text) RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS(SELECT 1 FROM people WHERE id=uid AND (owner OR admin))
    OR EXISTS(SELECT 1 FROM members WHERE mission=mid AND (principal=uid OR principal IN
        (SELECT group_id FROM group_members WHERE person=uid)))
$$;
CREATE TABLE IF NOT EXISTS push_deliveries (notification bigint REFERENCES notifications, endpoint text NOT NULL, PRIMARY KEY(notification,endpoint));
ALTER TABLE skills ADD COLUMN IF NOT EXISTS files jsonb NOT NULL DEFAULT '{}';
ALTER TABLE runs ADD COLUMN IF NOT EXISTS working_thread text REFERENCES threads;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS native_turn text;
ALTER TABLE messages ADD COLUMN IF NOT EXISTS links jsonb NOT NULL DEFAULT '[]';
ALTER TABLE routines ADD COLUMN IF NOT EXISTS directory text NOT NULL DEFAULT '.';
ALTER TABLE oidc_flows ADD COLUMN IF NOT EXISTS cli_return jsonb;
CREATE TABLE IF NOT EXISTS cli_codes (digest text PRIMARY KEY, person text REFERENCES people, challenge text NOT NULL, expires timestamptz NOT NULL);
ALTER TABLE people ADD COLUMN IF NOT EXISTS push_settings jsonb NOT NULL DEFAULT '{"mentions":true,"questions":true,"outcomes":true,"following":true}';
ALTER TABLE messages ADD COLUMN IF NOT EXISTS native_reply jsonb;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS next_input bigint REFERENCES messages;
ALTER TABLE skills ADD COLUMN IF NOT EXISTS description text NOT NULL DEFAULT '';

CREATE TABLE IF NOT EXISTS model_settings (singleton boolean PRIMARY KEY DEFAULT true CHECK(singleton), wip text NOT NULL);
INSERT INTO model_settings(wip) VALUES ('gpt-6-luna') ON CONFLICT DO NOTHING;
ALTER TABLE routines DROP COLUMN IF EXISTS hook_hash, DROP COLUMN IF EXISTS cron, DROP COLUMN IF EXISTS next_at;
CREATE TABLE IF NOT EXISTS webhooks (
    id text PRIMARY KEY, owner_id text NOT NULL REFERENCES people, name text NOT NULL
);
CREATE TABLE IF NOT EXISTS webhook_tokens (
    id text PRIMARY KEY, hook text NOT NULL REFERENCES webhooks ON DELETE CASCADE,
    name text NOT NULL, digest text NOT NULL UNIQUE, last_used timestamptz
);
CREATE TABLE IF NOT EXISTS webhook_deliveries (
    hook text REFERENCES webhooks ON DELETE CASCADE, sender text NOT NULL, delivery text NOT NULL,
    created timestamptz NOT NULL DEFAULT now(), PRIMARY KEY(hook,sender,delivery)
);
CREATE TABLE IF NOT EXISTS routine_triggers (
    id text PRIMARY KEY, routine text NOT NULL REFERENCES routines ON DELETE CASCADE,
    webhook text REFERENCES webhooks ON DELETE CASCADE, cron text, timezone text NOT NULL DEFAULT 'UTC', next_at timestamptz,
    CHECK ((webhook IS NULL) <> (cron IS NULL)), UNIQUE(routine,webhook)
);
CREATE INDEX IF NOT EXISTS trigger_due ON routine_triggers(next_at) WHERE next_at IS NOT NULL;
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS definition jsonb NOT NULL DEFAULT '{}';
