//! The proxy's local HTTPS endpoint, for the Codex setting that must be HTTPS.
//!
//! Codex fetches the usage `/status` shows from `{chatgpt_base_url}/wham/usage`
//! and refuses a `chatgpt_base_url` that is not HTTPS. The proxy therefore also
//! listens on HTTPS, one port above its own, with a certificate for 127.0.0.1
//! and localhost. A certificate authority is made once to sign it and its
//! private key is never stored, so the authority the system is told to trust
//! can sign nothing else. Codex is pointed at the HTTPS endpoint only on an
//! install that trusts that authority.

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, SanType,
};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

const CA_FILE: &str = "local_ca.pem";
const CERT_FILE: &str = "local_cert.pem";
const KEY_FILE: &str = "local_key.pem";

/// Name of the authority in the WSL trust store.
#[cfg(all(windows, not(test)))]
const WSL_CA_PATH: &str = "/usr/local/share/ca-certificates/switchy-local-proxy.crt";

/// Port the HTTPS endpoint listens on; 0 while it is not listening.
static LISTENING_PORT: AtomicU16 = AtomicU16::new(0);

fn dir() -> PathBuf {
    crate::config::get_app_config_dir().join("tls")
}

/// Makes the authority and the proxy's certificate the first time.
pub fn ensure() -> Result<(), String> {
    let dir = dir();
    if [CA_FILE, CERT_FILE, KEY_FILE]
        .iter()
        .all(|name| dir.join(name).exists())
    {
        return Ok(());
    }
    let (ca_pem, cert_pem, key_pem) = generate().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(CA_FILE), ca_pem).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(CERT_FILE), cert_pem).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(KEY_FILE), key_pem).map_err(|e| e.to_string())?;
    log::info!(
        "[local_tls] made the local HTTPS certificate in {}",
        dir.display()
    );
    forget_old_windows_authorities();
    Ok(())
}

/// (authority PEM, certificate PEM, certificate key PEM). The authority's key
/// is dropped here.
fn generate() -> Result<(String, String, String), rcgen::Error> {
    let year = chrono::Datelike::year(&chrono::Utc::now());
    let not_before = rcgen::date_time_ymd(year - 1, 1, 1);
    let not_after = rcgen::date_time_ymd(year + 20, 1, 1);

    let mut ca = CertificateParams::new(Vec::<String>::new())?;
    ca.distinguished_name
        .push(DnType::CommonName, "Switchy local proxy");
    ca.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    ca.not_before = not_before;
    ca.not_after = not_after;
    let ca_key = KeyPair::generate()?;
    let ca_cert = ca.self_signed(&ca_key)?;

    let mut leaf = CertificateParams::new(vec!["localhost".to_string()])?;
    leaf.subject_alt_names
        .push(SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)));
    leaf.distinguished_name
        .push(DnType::CommonName, "127.0.0.1");
    leaf.is_ca = IsCa::ExplicitNoCa;
    leaf.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    leaf.use_authority_key_identifier_extension = true;
    leaf.not_before = not_before;
    leaf.not_after = not_after;
    let leaf_key = KeyPair::generate()?;
    let leaf_cert = leaf.signed_by(&leaf_key, &ca_cert, &ca_key)?;

    Ok((ca_cert.pem(), leaf_cert.pem(), leaf_key.serialize_pem()))
}

/// The TLS acceptor serving the proxy's certificate.
pub fn acceptor() -> Result<tokio_rustls::TlsAcceptor, String> {
    ensure()?;
    let dir = dir();
    let certs = CertificateDer::pem_file_iter(dir.join(CERT_FILE))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let key = PrivateKeyDer::from_pem_file(dir.join(KEY_FILE)).map_err(|e| e.to_string())?;
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| e.to_string())?
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .map_err(|e| e.to_string())?;
    Ok(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
}

pub fn set_listening(port: u16) {
    LISTENING_PORT.store(port, Ordering::SeqCst);
}

/// Which Codex install a `chatgpt_base_url` is for.
pub enum Install<'a> {
    Windows,
    /// The WSL install whose config directory is this (`\\wsl$\<distro>\...`).
    Wsl(&'a std::path::Path),
}

/// The HTTPS `chatgpt_base_url` for `install`, or `None` when the endpoint is
/// not listening or the install does not trust its authority (which is
/// installed first where missing). Codex exits at startup on an untrusted one.
pub fn codex_chatgpt_base_url(install: Install) -> Option<String> {
    let port = LISTENING_PORT.load(Ordering::SeqCst);
    if port == 0 {
        return None;
    }
    let trusted = match install {
        Install::Windows => trust_on_windows(),
        Install::Wsl(config_dir) => wsl_distro(config_dir).is_some_and(|d| trust_in_wsl(&d)),
    };
    trusted.then(|| {
        format!(
            "https://127.0.0.1:{port}{}",
            super::codex_pool::CHATGPT_BACKEND_PATH_PREFIX
        )
    })
}

/// `Ubuntu-22.04` from `\\wsl$\Ubuntu-22.04\home\...` or
/// `\\wsl.localhost\Ubuntu-22.04\home\...`.
fn wsl_distro(config_dir: &std::path::Path) -> Option<String> {
    let text = config_dir.to_string_lossy().replace('/', "\\");
    let rest = text
        .strip_prefix("\\\\wsl$\\")
        .or_else(|| text.strip_prefix("\\\\wsl.localhost\\"))?;
    rest.split('\\')
        .next()
        .filter(|d| !d.is_empty())
        .map(str::to_string)
}

#[cfg(all(windows, not(test)))]
fn trust_on_windows() -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let ca = dir().join(CA_FILE);
    if ensure().is_err() {
        return false;
    }
    let result = std::process::Command::new("certutil")
        .args(["-addstore", "-f", "Root"])
        .arg(&ca)
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match result {
        Ok(out) if out.status.success() => true,
        Ok(out) => {
            log::warn!(
                "[local_tls] Windows did not take the local authority (certutil exit {:?}); Codex on Windows keeps its usage reads direct",
                out.status.code()
            );
            false
        }
        Err(e) => {
            log::warn!("[local_tls] could not run certutil: {e}");
            false
        }
    }
}

#[cfg(any(not(windows), test))]
fn trust_on_windows() -> bool {
    false
}

#[cfg(all(windows, not(test)))]
fn trust_in_wsl(distro: &str) -> bool {
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    if ensure().is_err() {
        return false;
    }
    let Ok(pem) = std::fs::read_to_string(dir().join(CA_FILE)) else {
        return false;
    };
    let pem = pem.replace("\r", "");
    // A line of the certificate's body, to find it in the system bundle.
    let Some(marker) = pem.lines().nth(1).filter(|l| !l.is_empty()) else {
        return false;
    };
    let bundle = "/etc/ssl/certs/ca-certificates.crt";
    // Sent as the shell's input: arguments to wsl.exe do not keep quoting.
    let script = [
        format!("grep -qF '{marker}' {bundle} || {{"),
        format!("mkdir -p \"$(dirname {WSL_CA_PATH})\""),
        format!("cat > {WSL_CA_PATH} <<'SWITCHY_CA'"),
        format!("{}SWITCHY_CA", pem),
        "update-ca-certificates >/dev/null 2>&1".to_string(),
        "}".to_string(),
        format!("grep -qF '{marker}' {bundle}"),
    ]
    .join("\n")
        + "\n";
    let child = std::process::Command::new("wsl.exe")
        .args(["-d", distro, "-u", "root", "--", "sh", "-s"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    let Ok(mut child) = child else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(script.as_bytes());
    }
    match child.wait() {
        Ok(status) if status.success() => true,
        _ => {
            log::warn!(
                "[local_tls] WSL {distro} does not trust the local authority; Codex there keeps its usage reads direct"
            );
            false
        }
    }
}

/// A new authority replaces the one Windows was told to trust before.
#[cfg(all(windows, not(test)))]
fn forget_old_windows_authorities() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("certutil")
        .args(["-delstore", "Root", "Switchy local proxy"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

#[cfg(any(not(windows), test))]
fn forget_old_windows_authorities() {}

#[cfg(any(not(windows), test))]
fn trust_in_wsl(_distro: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_distro_is_read_from_a_wsl_path() {
        assert_eq!(
            wsl_distro(std::path::Path::new(r"\\wsl$\Ubuntu-22.04\home\me\.codex")).as_deref(),
            Some("Ubuntu-22.04")
        );
        assert_eq!(
            wsl_distro(std::path::Path::new(
                r"\\wsl.localhost\Debian\home\me\.codex"
            ))
            .as_deref(),
            Some("Debian")
        );
        assert_eq!(
            wsl_distro(std::path::Path::new(r"C:\Users\me\.codex")),
            None
        );
    }

    /// A client that trusts only the authority accepts the proxy's certificate
    /// for 127.0.0.1, as Codex's rustls does once the system trusts it.
    #[tokio::test]
    async fn a_client_trusting_the_authority_accepts_the_certificate() {
        let (ca_pem, cert_pem, key_pem) = generate().expect("generate");
        let certs = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .expect("certs");
        let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).expect("key");
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let server = rustls::ServerConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .expect("versions")
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .expect("server config");
        let mut roots = rustls::RootCertStore::empty();
        for cert in CertificateDer::pem_slice_iter(ca_pem.as_bytes()) {
            roots.add(cert.expect("ca")).expect("add root");
        }
        let client = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("versions")
            .with_root_certificates(roots)
            .with_no_client_auth();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
        let served = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            acceptor.accept(stream).await.map(|_| ())
        });
        let connector = tokio_rustls::TlsConnector::from(Arc::new(client));
        let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let name = rustls::pki_types::ServerName::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST).into());
        connector
            .connect(name, stream)
            .await
            .expect("the client accepts the certificate");
        served.await.expect("join").expect("server handshake");
    }
}
