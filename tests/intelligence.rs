use episteme::domain::{
    AnalysisProvenance, ClaimKind, DocumentClassification, DocumentIntelligence,
    DocumentSourceType, EntityKind, EvidenceReference, IntelligenceClaim, IntelligenceEntity,
    IntelligenceSummary, SemanticRelation, SemanticRelationType, SourceDigest,
};

#[test]
fn intelligence_canonicalizes_ids_and_rejects_ungrounded_graphs() {
    let source = "Rust implements memory safety through ownership.";
    let digest = SourceDigest::from_bytes(source.as_bytes());
    let entity = IntelligenceEntity::new(
        "Rust",
        EntityKind::Technology,
        "A systems programming language",
        vec![evidence(source)],
    )
    .expect("entity should normalize");
    let claim = IntelligenceClaim::new(
        "Rust implements memory safety through ownership.",
        ClaimKind::Fact,
        95,
        vec![evidence(source)],
    )
    .expect("claim should normalize");
    let relation = SemanticRelation::new(
        entity.id.clone(),
        claim.id.clone(),
        SemanticRelationType::Implements,
        90,
        vec![evidence(source)],
        &digest,
    )
    .expect("relation should normalize");
    let intelligence = DocumentIntelligence {
        source_digest: digest.clone(),
        classification: classification(),
        summary: IntelligenceSummary {
            text: "Rust uses ownership for memory safety.".to_owned(),
            key_points: vec!["Ownership supports memory safety".to_owned()],
            evidence: vec![evidence(source)],
        },
        claims: vec![claim.clone()],
        entities: vec![entity.clone()],
        relations: vec![relation],
        analysis: provenance(),
    };

    assert!(intelligence.clone().validate_against(source).is_ok());
    assert_eq!(
        entity.id,
        IntelligenceEntity::stable_id(EntityKind::Technology, "rust")
    );
    assert_eq!(claim.id, IntelligenceClaim::stable_id(claim.text.as_str()));

    let mut invalid = intelligence;
    invalid.relations[0].target_id = "missing-node".to_owned();
    assert!(invalid.validate_against(source).is_err());
}

fn evidence(source: &str) -> EvidenceReference {
    EvidenceReference {
        quote: source.to_owned(),
        location: format!("characters 0-{}", source.chars().count() - 1),
    }
}

fn classification() -> DocumentClassification {
    DocumentClassification {
        title: "Rust Ownership".to_owned(),
        authors: Vec::new(),
        source_type: DocumentSourceType::Documentation,
        language_code: "en".to_owned(),
        topics: vec!["rust".to_owned(), "ownership".to_owned()],
    }
}

fn provenance() -> AnalysisProvenance {
    AnalysisProvenance {
        function: "DocumentIntelligence".to_owned(),
        client: "local".to_owned(),
        model: "local-model".to_owned(),
        pipeline_version: "intelligence-v1".to_owned(),
        processed_at: "2026-09-19T00:00:00Z".to_owned(),
    }
}
