CREATE TABLE IF NOT EXISTS ingestion_runs (
    source_digest VARCHAR PRIMARY KEY,
    source_name VARCHAR NOT NULL,
    stage VARCHAR NOT NULL,
    note_path VARCHAR,
    archive_path VARCHAR,
    error VARCHAR
);
