//! Tests rejection of research drafts without source-grounded evidence.

use episteme::domain::{EvidenceReference, ResearchDraft};

#[test]
fn analysis_rejects_ungrounded_research_draft() {
    let draft = ResearchDraft {
        title: "A paper".to_owned(),
        citation: "Author, A paper".to_owned(),
        topics: vec!["research".to_owned()],
        summary: "Summary".to_owned(),
        key_ideas: vec!["Idea".to_owned()],
        implementation_notes: Vec::new(),
        critique: "Critique".to_owned(),
        evidence: Vec::new(),
    };

    assert!(draft.validate().is_err());

    let fabricated = ResearchDraft {
        title: "A paper".to_owned(),
        citation: "Author, A paper".to_owned(),
        topics: vec!["research".to_owned()],
        summary: "Summary".to_owned(),
        key_ideas: vec!["Idea".to_owned()],
        implementation_notes: Vec::new(),
        critique: "Critique".to_owned(),
        evidence: vec![EvidenceReference {
            quote: "fabricated quotation".to_owned(),
            location: "paragraph 1".to_owned(),
        }],
    };
    assert!(fabricated.validate_against("actual source text").is_err());
}
