//! TLS for IMAP (rustls with the ring provider). Certificates are checked
//! against the system trust store (falling back to the Mozilla roots when the
//! system store is empty); there is no way to bypass validation (§2.4). The
//! verifier remembers the rejected certificate's SHA-256 fingerprint for the
//! error's details expander.

use std::fmt::Write as _;
use std::sync::{Arc, Mutex, OnceLock};

use sha2::{Digest, Sha256};
use tokio_rustls::rustls::client::WebPkiServerVerifier;
use tokio_rustls::rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use tokio_rustls::rustls::crypto::{CryptoProvider, ring};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{self, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};

fn provider() -> Arc<CryptoProvider> {
    static P: OnceLock<Arc<CryptoProvider>> = OnceLock::new();
    P.get_or_init(|| Arc::new(ring::default_provider())).clone()
}

fn roots() -> Arc<RootCertStore> {
    static R: OnceLock<Arc<RootCertStore>> = OnceLock::new();
    R.get_or_init(|| {
        let mut store = RootCertStore::empty();
        let native = rustls_native_certs::load_native_certs();
        for cert in native.certs {
            let _ = store.add(cert);
        }
        if store.is_empty() {
            store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        }
        Arc::new(store)
    })
    .clone()
}

/// Delegates to the WebPKI verifier and records the end-entity fingerprint
/// when validation fails.
#[derive(Debug)]
struct RecordingVerifier {
    inner: Arc<WebPkiServerVerifier>,
    rejected: Arc<Mutex<Option<String>>>,
}

impl ServerCertVerifier for RecordingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let result = self.inner.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now);
        if let Err(e) = &result {
            let mut fp = String::new();
            for (i, b) in Sha256::digest(end_entity.as_ref()).iter().enumerate() {
                if i > 0 {
                    fp.push(':');
                }
                let _ = write!(fp, "{b:02X}");
            }
            *self.rejected.lock().unwrap_or_else(|p| p.into_inner()) = Some(format!("Reason: {e}\nSHA-256 fingerprint: {fp}"));
        }
        result
    }

    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// A client config for one connection plus the slot where a rejected
/// certificate's details land.
pub fn client_config() -> (Arc<ClientConfig>, Arc<Mutex<Option<String>>>) {
    let rejected = Arc::new(Mutex::new(None));
    let inner = WebPkiServerVerifier::builder_with_provider(roots(), provider())
        .build()
        .expect("root store is never empty");
    let verifier = Arc::new(RecordingVerifier { inner, rejected: rejected.clone() });
    let config = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    (Arc::new(config), rejected)
}
