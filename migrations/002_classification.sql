CREATE TABLE IF NOT EXISTS document_classifications (
    source_digest VARCHAR PRIMARY KEY,
    source_name VARCHAR NOT NULL,
    extraction_method VARCHAR NOT NULL,
    title VARCHAR NOT NULL,
    authors_json VARCHAR NOT NULL,
    source_type VARCHAR NOT NULL,
    language VARCHAR NOT NULL,
    topics_json VARCHAR NOT NULL,
    analysis_function VARCHAR NOT NULL,
    analysis_client VARCHAR NOT NULL,
    analysis_model VARCHAR NOT NULL,
    pipeline_version VARCHAR NOT NULL,
    processed_at VARCHAR NOT NULL
);
