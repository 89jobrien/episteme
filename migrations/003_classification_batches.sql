CREATE TABLE IF NOT EXISTS document_classification_versions (
    source_digest VARCHAR NOT NULL,
    analysis_model VARCHAR NOT NULL,
    policy_version VARCHAR NOT NULL,
    source_name VARCHAR NOT NULL,
    extraction_method VARCHAR NOT NULL,
    title VARCHAR NOT NULL,
    authors_json VARCHAR NOT NULL,
    source_type VARCHAR NOT NULL,
    language_code VARCHAR NOT NULL,
    topics_json VARCHAR NOT NULL,
    analysis_function VARCHAR NOT NULL,
    analysis_client VARCHAR NOT NULL,
    processed_at VARCHAR NOT NULL,
    PRIMARY KEY (source_digest, analysis_model, policy_version)
);

CREATE TABLE IF NOT EXISTS classification_batches (
    batch_id VARCHAR PRIMARY KEY,
    started_at VARCHAR NOT NULL,
    completed_at VARCHAR,
    total UBIGINT,
    succeeded UBIGINT,
    cached UBIGINT,
    failed_retryable UBIGINT,
    failed_terminal UBIGINT
);

CREATE TABLE IF NOT EXISTS classification_attempts (
    batch_id VARCHAR NOT NULL,
    source_path VARCHAR NOT NULL,
    attempt UINTEGER NOT NULL,
    source_digest VARCHAR,
    status VARCHAR NOT NULL,
    model VARCHAR,
    extracted_characters UBIGINT,
    duration_ms UBIGINT NOT NULL,
    failure_code VARCHAR,
    failure_message VARCHAR,
    updated_at VARCHAR NOT NULL,
    PRIMARY KEY (batch_id, source_path, attempt)
);

INSERT INTO document_classification_versions
SELECT
    source_digest,
    analysis_model,
    'legacy-v1',
    source_name,
    extraction_method,
    title,
    authors_json,
    lower(replace(source_type, ' ', '_')),
    lower(language),
    topics_json,
    analysis_function,
    analysis_client,
    processed_at
FROM document_classifications
WHERE lower(replace(source_type, ' ', '_')) IN (
    'research_paper', 'report', 'article', 'documentation', 'website', 'source_code',
    'repository', 'specification', 'tutorial', 'personal_profile', 'other'
)
AND regexp_matches(lower(language), '^[a-z]{2}$')
AND lower(title) NOT IN ('string', 'document', 'text', 'title', 'unknown', 'untitled')
AND extraction_method IN ('pdf_text', 'pdf_ocr', 'html_pandoc', 'image_ocr')
AND source_name <> ''
AND strpos(source_name, '/') = 0
AND strpos(source_name, '\\') = 0
AND json_valid(authors_json)
AND json_valid(topics_json)
AND json_array_length(topics_json) > 0
AND lower(authors_json) NOT LIKE '%source_document%'
AND lower(topics_json) NOT LIKE '%string[]%'
ON CONFLICT DO NOTHING;
