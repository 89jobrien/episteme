use episteme::config::{LocalModelEndpoint, SettingsError};

#[test]
fn settings_reject_non_loopback_model_endpoint() {
    let error = LocalModelEndpoint::new("https://api.openai.com/v1", "remote")
        .expect_err("a remote endpoint must be rejected");

    assert!(matches!(error, SettingsError::ModelEndpointNotLoopback(_)));
    assert!(LocalModelEndpoint::new("http://127.0.0.1:11434/v1", "local").is_ok());
}
