CREATE TABLE speech_cue_edits (
    job INTEGER NOT NULL,
    ordinal INTEGER NOT NULL,
    cue INTEGER NOT NULL CHECK(cue >= 0),
    revision INTEGER NOT NULL CHECK(revision > 0),
    text TEXT NOT NULL CHECK(length(CAST(text AS BLOB)) <= 262144),
    PRIMARY KEY(job, ordinal, cue),
    FOREIGN KEY(job, ordinal) REFERENCES segments(job, ordinal)
) STRICT, WITHOUT ROWID;
