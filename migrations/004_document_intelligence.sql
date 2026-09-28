CREATE TABLE IF NOT EXISTS document_intelligence_versions (
    source_digest VARCHAR NOT NULL,
    analysis_model VARCHAR NOT NULL,
    policy_version VARCHAR NOT NULL,
    intelligence_json VARCHAR NOT NULL,
    processed_at VARCHAR NOT NULL,
    PRIMARY KEY (source_digest, analysis_model, policy_version)
);

CREATE TABLE IF NOT EXISTS intelligence_claims (
    source_digest VARCHAR NOT NULL,
    analysis_model VARCHAR NOT NULL,
    policy_version VARCHAR NOT NULL,
    claim_id VARCHAR NOT NULL,
    claim_text VARCHAR NOT NULL,
    claim_kind VARCHAR NOT NULL,
    confidence_percent UTINYINT NOT NULL,
    evidence_json VARCHAR NOT NULL,
    PRIMARY KEY (source_digest, analysis_model, policy_version, claim_id)
);

CREATE TABLE IF NOT EXISTS intelligence_entities (
    entity_id VARCHAR PRIMARY KEY,
    entity_kind VARCHAR NOT NULL,
    canonical_name VARCHAR NOT NULL,
    aliases_json VARCHAR NOT NULL,
    description VARCHAR NOT NULL
);

CREATE TABLE IF NOT EXISTS document_intelligence_entities (
    source_digest VARCHAR NOT NULL,
    analysis_model VARCHAR NOT NULL,
    policy_version VARCHAR NOT NULL,
    entity_id VARCHAR NOT NULL,
    evidence_json VARCHAR NOT NULL,
    PRIMARY KEY (source_digest, analysis_model, policy_version, entity_id)
);

CREATE TABLE IF NOT EXISTS semantic_relations (
    source_digest VARCHAR NOT NULL,
    analysis_model VARCHAR NOT NULL,
    policy_version VARCHAR NOT NULL,
    relation_id VARCHAR NOT NULL,
    source_node_id VARCHAR NOT NULL,
    target_node_id VARCHAR NOT NULL,
    relation_type VARCHAR NOT NULL,
    confidence_percent UTINYINT NOT NULL,
    evidence_json VARCHAR NOT NULL,
    PRIMARY KEY (source_digest, analysis_model, policy_version, relation_id)
);
