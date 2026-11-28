CREATE TABLE session (
    id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 1 AND 128),
    account TEXT NOT NULL CHECK(length(account) BETWEEN 1 AND 128),
    provider TEXT NOT NULL CHECK(length(provider) BETWEEN 1 AND 64),
    track TEXT NOT NULL CHECK(length(track) BETWEEN 1 AND 512),
    duration_ms INTEGER NOT NULL CHECK(duration_ms > 0),
    outcome TEXT CHECK(outcome IN ('natural_end','skipped','stopped','failed','interrupted'))
) STRICT;
CREATE INDEX session_account ON session(account,id);
CREATE TABLE segment (
    session_id TEXT NOT NULL REFERENCES session(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL CHECK(seq > 0),
    start_utc_ms INTEGER NOT NULL CHECK(start_utc_ms >= 0),
    end_utc_ms INTEGER NOT NULL CHECK(end_utc_ms > start_utc_ms),
    PRIMARY KEY(session_id,seq)
) STRICT;
CREATE INDEX segment_time ON segment(start_utc_ms,end_utc_ms);
CREATE TRIGGER segment_order BEFORE INSERT ON segment BEGIN
    SELECT CASE WHEN (SELECT outcome FROM session WHERE id=NEW.session_id) IS NOT NULL
      THEN RAISE(ABORT,'closed session') END;
    SELECT CASE WHEN NEW.seq != COALESCE((SELECT MAX(seq)+1 FROM segment
      WHERE session_id=NEW.session_id),1) THEN RAISE(ABORT,'sequence gap') END;
    SELECT CASE WHEN NEW.start_utc_ms < COALESCE((SELECT end_utc_ms FROM segment
      WHERE session_id=NEW.session_id ORDER BY seq DESC LIMIT 1),0)
      THEN RAISE(ABORT,'overlapping session intervals') END;
END;
PRAGMA application_id=1095454037;
PRAGMA user_version=1;
