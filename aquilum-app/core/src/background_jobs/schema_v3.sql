CREATE TABLE speech_search_cues (
    id INTEGER PRIMARY KEY,
    job INTEGER NOT NULL,
    ordinal INTEGER NOT NULL,
    cue INTEGER NOT NULL,
    start_ms INTEGER NOT NULL,
    revision INTEGER NOT NULL,
    text TEXT NOT NULL,
    UNIQUE(job, ordinal, cue),
    FOREIGN KEY(job, ordinal) REFERENCES segments(job, ordinal)
) STRICT;

CREATE VIRTUAL TABLE speech_fts USING fts5(
    text, content='speech_search_cues', content_rowid='id',
    tokenize='unicode61 remove_diacritics 2', prefix='2 3 4'
);

CREATE TRIGGER speech_fts_insert AFTER INSERT ON speech_search_cues BEGIN
    INSERT INTO speech_fts(rowid, text) VALUES(new.id, new.text);
END;

CREATE TRIGGER speech_fts_edit AFTER UPDATE OF text ON speech_search_cues BEGIN
    INSERT INTO speech_fts(speech_fts, rowid, text) VALUES('delete', old.id, old.text);
    INSERT INTO speech_fts(rowid, text) VALUES(new.id, new.text);
END;

CREATE VIEW speech_search_projection AS
SELECT s.job AS job, s.ordinal AS ordinal, c.key AS cue,
       json_extract(CASE WHEN c.type='object' THEN c.value ELSE '{}' END, '$.start_ms') AS start_ms,
       coalesce(e.revision, 0) AS revision,
       coalesce(e.text, json_extract(CASE WHEN c.type='object' THEN c.value ELSE '{}' END, '$.text')) AS text
FROM segments s JOIN jobs j ON j.id = s.job AND j.kind = 'speech'
JOIN json_each(CASE WHEN json_valid(CAST(s.payload AS TEXT)) THEN
                    CASE WHEN json_type(CAST(s.payload AS TEXT)) = 'array' THEN CAST(s.payload AS TEXT) ELSE '[]' END
               ELSE '[]' END) c
LEFT JOIN speech_cue_edits e ON e.job = s.job AND e.ordinal = s.ordinal AND e.cue = c.key
WHERE c.type = 'object'
  AND json_type(CASE WHEN c.type='object' THEN c.value ELSE '{}' END, '$.start_ms') = 'integer'
  AND json_type(CASE WHEN c.type='object' THEN c.value ELSE '{}' END, '$.text') = 'text'
  AND json_extract(CASE WHEN c.type='object' THEN c.value ELSE '{}' END, '$.start_ms') >= 0;

CREATE TRIGGER speech_index_segment AFTER INSERT ON segments
WHEN (SELECT kind FROM jobs WHERE id=new.job) = 'speech' BEGIN
    INSERT INTO speech_search_cues(job, ordinal, cue, start_ms, revision, text)
      SELECT job, ordinal, cue, start_ms, revision, text FROM speech_search_projection
       WHERE job = new.job AND ordinal = new.ordinal;
END;

CREATE TRIGGER speech_index_first_correction AFTER INSERT ON speech_cue_edits BEGIN
    UPDATE speech_search_cues SET text=new.text, revision=new.revision
      WHERE job=new.job AND ordinal=new.ordinal AND cue=new.cue;
END;

CREATE TRIGGER speech_index_next_correction AFTER UPDATE OF text ON speech_cue_edits BEGIN
    UPDATE speech_search_cues SET text=new.text, revision=new.revision
      WHERE job=new.job AND ordinal=new.ordinal AND cue=new.cue;
END;

INSERT INTO speech_search_cues(job, ordinal, cue, start_ms, revision, text)
SELECT job, ordinal, cue, start_ms, revision, text FROM speech_search_projection;
