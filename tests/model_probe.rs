use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use episteme::adapters::HttpModelProbe;
use episteme::config::{ClassifierProfile, LocalModelEndpoint};
use episteme::ports::ModelProbe;

#[tokio::test]
async fn model_probe_reads_local_models_without_following_redirects()
-> Result<(), Box<dyn std::error::Error>> {
    let (endpoint, server) = serve_once(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"data\":[{\"id\":\"large-local-model\"}]}",
    )?;
    let profile = ClassifierProfile {
        name: "large".to_owned(),
        endpoint: LocalModelEndpoint::new(&format!("http://{endpoint}/v1"), "large-local-model")?,
        maximum_input_characters: 120_000,
    };
    let probe = HttpModelProbe::new(Duration::from_secs(2))?;

    let availability = probe.probe(&profile).await?;

    assert_eq!(availability.advertised_models, ["large-local-model"]);
    server.join().map_err(|_| "model server panicked")?;

    let (redirect_endpoint, redirect_server) = serve_once(
        "HTTP/1.1 302 Found\r\nLocation: https://example.com/v1/models\r\nContent-Length: 0\r\n\r\n",
    )?;
    let redirect_profile = ClassifierProfile {
        name: "redirect".to_owned(),
        endpoint: LocalModelEndpoint::new(
            &format!("http://{redirect_endpoint}/v1"),
            "large-local-model",
        )?,
        maximum_input_characters: 120_000,
    };

    let redirected = probe
        .probe(&redirect_profile)
        .await
        .expect_err("redirecting model probes must fail closed");

    assert_eq!(
        redirected.code,
        episteme::domain::ClassificationFailureCode::EndpointUnavailable
    );
    redirect_server
        .join()
        .map_err(|_| "redirect server panicked")?;
    Ok(())
}

fn serve_once(
    response: &'static str,
) -> Result<(std::net::SocketAddr, thread::JoinHandle<()>), std::io::Error> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let server = thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut request = [0_u8; 2048];
        let _ = stream.read(&mut request);
        let _ = stream.write_all(response.as_bytes());
    });
    Ok((address, server))
}
