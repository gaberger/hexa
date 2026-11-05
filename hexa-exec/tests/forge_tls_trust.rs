//! The forge client trusts the operating system's certificate store, not only
//! the public roots it ships with (found 2026-11-05: strata's `git.local` is a
//! Forgejo behind Caddy's local CA, which the owner's keychain trusts and
//! `curl` accepts, while `hexa forge status` said "could not connect").
//!
//! A local TLS server presents a certificate signed by a private CA. The real
//! `hexa forge status` runs as a subprocess, so the environment (the CA file
//! the system store reads) is set per child and the test never writes its own
//! process environment (ADR-2609131749).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

const TOKEN: &str = "tls-test-token-4f8a1c";

fn hexa_bin() -> PathBuf {
    if let Ok(p) = std::env::var("HEXA_BIN") {
        return PathBuf::from(p);
    }
    let exe = std::env::current_exe().expect("current exe");
    let mut dir = exe.parent().expect("test binary dir").to_path_buf();
    if dir.ends_with("deps") {
        dir.pop();
    }
    let p = dir.join("hexa");
    assert!(p.exists(), "build hexa-cli first (same profile as this test): {}", p.display());
    p
}

/// A private CA (PEM, to trust) and a `localhost` server certificate it signed.
fn private_ca() -> (String, Arc<ServerConfig>) {
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::new()).unwrap();
    ca_params.distinguished_name.push(DnType::CommonName, "hexa test local authority");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca = ca_params.self_signed(&ca_key).unwrap();

    let leaf_key = KeyPair::generate().unwrap();
    let leaf_params = CertificateParams::new(vec!["localhost".to_string()]).unwrap();
    let leaf = leaf_params.signed_by(&leaf_key, &ca, &ca_key).unwrap();

    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
    let chain = vec![CertificateDer::from(leaf.der().to_vec())];
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .unwrap();
    (ca.pem(), Arc::new(cfg))
}

fn serve_one(cfg: Arc<ServerConfig>, tcp: TcpStream) {
    let Ok(conn) = ServerConnection::new(cfg) else { return };
    let mut tls = StreamOwned::new(conn, tcp);
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        match tls.read(&mut chunk) {
            Ok(0) | Err(_) => return, // a client that rejected our certificate ends here
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let first = String::from_utf8_lossy(&buf).lines().next().unwrap_or("").to_string();
    let body = if first.contains("/api/v1/version") {
        r#"{"version":"16.0.6+gitea-1.22.0"}"#
    } else if first.contains("/api/v1/user") {
        r#"{"login":"gary"}"#
    } else {
        r#"{"message":"not found"}"#
    };
    let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = tls.write_all(resp.as_bytes());
    let _ = tls.flush();
    tls.conn.send_close_notify();
    let _ = tls.flush();
}

/// Serve TLS on a free loopback port until the test ends; returns the port.
fn spawn_server(cfg: Arc<ServerConfig>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for tcp in listener.incoming().flatten() {
            let cfg = cfg.clone();
            std::thread::spawn(move || serve_one(cfg, tcp));
        }
    });
    port
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git").args(args).current_dir(dir).status().expect("git runs").success();
    assert!(ok, "git {args:?}");
}

/// Run `hexa forge status` in a repository whose origin is the TLS server.
/// `ca_file` is handed to the child only, as the system store's override.
fn forge_status(port: u16, ca_file: Option<&Path>) -> (bool, String) {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(home.join(".hexa")).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["remote", "add", "origin", &format!("https://localhost:{port}/me/strata.git")]);
    std::fs::write(
        home.join("forges.json"),
        format!(r#"[{{"host":"localhost:{port}","kind":"forgejo","token_env":"FORGE_TLS_TEST_TOKEN"}}]"#),
    )
    .unwrap();
    let mut cmd = Command::new(hexa_bin());
    cmd.args(["forge", "status"])
        .current_dir(&repo)
        .env("HEXA_HOME", &home)
        .env("HOME", &home)
        .env("FORGE_TLS_TEST_TOKEN", TOKEN)
        .env_remove("SSL_CERT_FILE")
        .env_remove("SSL_CERT_DIR");
    if let Some(ca) = ca_file {
        cmd.env("SSL_CERT_FILE", ca);
    }
    let out = cmd.output().expect("run hexa");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

#[test]
fn a_forge_whose_certificate_the_system_store_trusts_is_reached() {
    let (ca_pem, cfg) = private_ca();
    let port = spawn_server(cfg);
    let dir = tempfile::tempdir().unwrap();
    let ca_file = dir.path().join("ca.pem");
    std::fs::write(&ca_file, ca_pem).unwrap();
    let (ok, out) = forge_status(port, Some(&ca_file));
    assert!(ok, "a private CA the system trusts is accepted:\n{out}");
    assert!(out.contains("16.0.6+gitea-1.22.0"), "the server answered: {out}");
    assert!(out.contains("credential accepted: gary"), "the credential was accepted: {out}");
    assert!(!out.contains(TOKEN), "the token never appears in output: {out}");
}

#[test]
fn a_certificate_nobody_trusts_is_named_as_such_not_as_a_dead_server() {
    let (_ca_pem, cfg) = private_ca();
    let port = spawn_server(cfg);
    let (ok, out) = forge_status(port, None);
    assert!(!ok, "an untrusted certificate is not accepted:\n{out}");
    assert!(out.contains("certificate"), "the failure says it is the certificate: {out}");
    assert!(!out.contains("could not connect"), "not the misleading 'could not connect': {out}");
    assert!(!out.contains(TOKEN), "the token never appears in output: {out}");
}
