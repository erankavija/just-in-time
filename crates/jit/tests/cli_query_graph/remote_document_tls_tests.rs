//! Coverage for jit:3398bc19 REQ-05: the narrowed `ureq` feature set
//! (`default-features = false`, `["rustls", "gzip"]`, no `native-tls`) must
//! still perform real HTTPS remote-document access and transparently decode a
//! gzip-compressed response, using exactly the rustls backend.
//!
//! `crate::commands::document::validate_external_url` (the only remote-
//! document call site in this crate) only issues HEAD requests, which never
//! carry a compressed body, so it cannot exercise gzip decoding itself. These
//! tests instead drive a `ureq::Agent` configured exactly like that call site
//! (`ureq::Agent::new_with_defaults()`, i.e. the crate's default TLS/gzip
//! feature selection) against a real local TLS server, proving the selected
//! feature set — not jit's own business logic — supports both behaviors.
//!
//! The server is a raw `rustls::ServerConnection` over a loopback
//! `TcpListener`, certified with an ephemeral self-signed certificate from
//! `rcgen` (test-only dev-dependency; never touches the network). The client
//! disables certificate verification via `ureq`'s own `TlsConfig` — the
//! documented, public escape hatch for exactly this case — so the test
//! exercises a genuine TLS 1.3 handshake and record layer without needing a
//! certificate a real root store would trust.

use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

/// Build a `rustls::ServerConfig` certified for `localhost` with a freshly
/// generated self-signed certificate, using the `ring` crypto provider (the
/// same backend `ureq`'s rustls feature selects, so no second crypto backend
/// is pulled in for this test).
fn test_server_config() -> Arc<ServerConfig> {
    let certified = generate_simple_self_signed(vec!["localhost".to_string()])
        .expect("generate ephemeral self-signed certificate");
    let cert_der: CertificateDer<'static> = certified.cert.der().clone();
    let key_der: PrivateKeyDer<'static> = certified.signing_key.into();

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("ring provider supports the default TLS protocol versions")
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .expect("self-signed cert and key must match");
    Arc::new(config)
}

/// Drain one HTTP request off `stream` (headers only; GET requests carry no
/// body), tolerating it arriving across more than one TLS record.
fn drain_request(stream: &mut impl Read) {
    let mut buf = [0u8; 4096];
    let mut seen = Vec::new();
    for _ in 0..16 {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                seen.extend_from_slice(&buf[..n]);
                if seen.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

/// Spawn a one-shot local HTTPS server that answers the single connection it
/// accepts with a `200 OK` response carrying `body` and `extra_headers`
/// verbatim, then returns the `https://localhost:<port>/` base URL to reach
/// it. `Connection: close` and an explicit `Content-Length` keep the exchange
/// to exactly one request/response with no keep-alive bookkeeping needed.
fn spawn_https_server(body: Vec<u8>, extra_headers: &'static str) -> String {
    let config = test_server_config();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback TLS listener");
    let addr = listener.local_addr().expect("loopback listener local addr");

    std::thread::spawn(move || {
        let Ok((sock, _)) = listener.accept() else {
            return;
        };
        sock.set_nodelay(true).ok();
        let conn = match ServerConnection::new(config) {
            Ok(conn) => conn,
            Err(_) => return,
        };
        let mut tls: StreamOwned<ServerConnection, TcpStream> = StreamOwned::new(conn, sock);

        drain_request(&mut tls);

        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n",
            body.len()
        );
        let _ = tls.write_all(response.as_bytes());
        let _ = tls.write_all(&body);
        let _ = tls.flush();
    });

    format!("https://localhost:{}/", addr.port())
}

/// `ureq::Agent` configured the way `validate_external_url` configures its
/// agent (`Agent::new_with_defaults()`), except with certificate verification
/// disabled — the local server's certificate is self-signed and not chained
/// to any root store. This changes nothing about which TLS backend or which
/// content-decoding feature runs: the handshake, record layer, and gzip
/// decoder are exactly what `Agent::new_with_defaults()` would use against a
/// publicly-trusted HTTPS server.
fn agent_trusting_test_server() -> ureq::Agent {
    let tls_config = ureq::tls::TlsConfig::builder()
        .disable_verification(true)
        .build();
    let config = ureq::Agent::config_builder().tls_config(tls_config).build();
    ureq::Agent::new_with_config(config)
}

#[test]
fn test_https_remote_document_access_succeeds_over_rustls() {
    let body = b"remote document contents".to_vec();
    let url = spawn_https_server(body.clone(), "Content-Type: text/plain\r\n");

    let agent = agent_trusting_test_server();
    let mut response = agent
        .get(&url)
        .call()
        .expect("HTTPS GET against the local rustls test server must succeed");

    assert_eq!(response.status().as_u16(), 200);
    let received = response
        .body_mut()
        .read_to_string()
        .expect("read response body");
    assert_eq!(received.as_bytes(), body.as_slice());
}

#[test]
fn test_https_remote_document_access_decodes_gzip_response_transparently() {
    use flate2::write::GzEncoder;
    use flate2::Compression;

    let plaintext = b"remote document contents, compressed in transit".to_vec();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&plaintext)
        .expect("write to gzip encoder");
    let compressed = encoder.finish().expect("finish gzip stream");
    // The server really did send compressed bytes, not a same-length coincidence.
    assert_ne!(compressed, plaintext);

    let url = spawn_https_server(
        compressed,
        "Content-Type: text/plain\r\nContent-Encoding: gzip\r\n",
    );

    let agent = agent_trusting_test_server();
    let mut response = agent
        .get(&url)
        .call()
        .expect("HTTPS GET for a gzip-encoded response must succeed");

    assert_eq!(response.status().as_u16(), 200);
    let received = response
        .body_mut()
        .read_to_string()
        .expect("ureq must transparently gzip-decode the body (the `gzip` feature)");
    assert_eq!(received.as_bytes(), plaintext.as_slice());
}
