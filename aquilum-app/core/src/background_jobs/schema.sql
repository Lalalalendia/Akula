CREATE TABLE jobs (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    source TEXT NOT NULL,
    revision TEXT NOT NULL,
    recipe TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'queued'
        CHECK(state IN ('queued','running','paused','cancelled','failed','completed')),
    generation INTEGER NOT NULL DEFAULT 0 CHECK(generation >= 0),
    lease_until INTEGER,
    committed INTEGER NOT NULL DEFAULT 0 CHECK(committed >= 0),
    total INTEGER CHECK(total >= committed),
    error TEXT,
    CHECK((state = 'running') = (lease_until IS NOT NULL)),
    UNIQUE(kind, source, revision, recipe)
) STRICT;
CREATE INDEX jobs_claim ON jobs(kind, state, id);

CREATE TABLE segments (
    job INTEGER NOT NULL REFERENCES jobs(id),
    ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
    start_ms INTEGER NOT NULL CHECK(start_ms >= 0),
    end_ms INTEGER NOT NULL CHECK(end_ms > start_ms),
    payload BLOB NOT NULL CHECK(length(payload) <= 262144),
    PRIMARY KEY(job, ordinal)
) STRICT, WITHOUT ROWID;
