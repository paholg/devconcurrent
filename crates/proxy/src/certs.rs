//! mkcert-rooted CA loaded into the proxy at startup and used to mint
//! per-service leaf certs on demand. The CA private key never leaves this
//! process: only the resulting cert + leaf-key PEMs are uploaded into
//! sidecars.
//!
//! Sidecars don't trust each other and don't hold the CA key, so a compromise
//! of one sidecar doesn't yield a CA.

use std::path::Path;
use std::sync::Arc;

use eyre::{Context, Result, eyre};
use rcgen::{CertificateParams, Issuer, KeyPair, KeyUsagePurpose};

const ROOT_CA_PEM: &str = "rootCA.pem";
const ROOT_CA_KEY_PEM: &str = "rootCA-key.pem";

/// Holds the mkcert CA cert + key in memory. Cheap to clone.
#[derive(Clone)]
pub struct CaHolder(Arc<Issuer<'static, KeyPair>>);

impl CaHolder {
    /// Load `rootCA.pem` + `rootCA-key.pem` from `dir`.
    pub fn load(dir: &Path) -> Result<Self> {
        let cert_pem = std::fs::read_to_string(dir.join(ROOT_CA_PEM))
            .wrap_err_with(|| format!("read {}", dir.join(ROOT_CA_PEM).display()))?;
        let key_pem = std::fs::read_to_string(dir.join(ROOT_CA_KEY_PEM))
            .wrap_err_with(|| format!("read {}", dir.join(ROOT_CA_KEY_PEM).display()))?;
        let key = KeyPair::from_pem(&key_pem).wrap_err("parse CA key")?;
        let issuer = Issuer::from_ca_cert_pem(&cert_pem, key).wrap_err("parse CA cert")?;
        Ok(Self(Arc::new(issuer)))
    }

    /// Mint a leaf cert with a single SAN equal to `hostname`, signed by the
    /// loaded CA. Returns `(cert_pem, key_pem)`.
    pub fn mint(&self, hostname: &str) -> Result<(String, String)> {
        let mut params = CertificateParams::new(vec![hostname.to_string()])
            .wrap_err("build leaf cert params")?;
        params.distinguished_name.push(
            rcgen::DnType::CommonName,
            rcgen::DnValue::Utf8String(hostname.to_string()),
        );
        params.key_usages.push(KeyUsagePurpose::DigitalSignature);
        params.key_usages.push(KeyUsagePurpose::KeyEncipherment);
        params
            .extended_key_usages
            .push(rcgen::ExtendedKeyUsagePurpose::ServerAuth);
        let leaf_key = KeyPair::generate().wrap_err("generate leaf key")?;
        let leaf = params
            .signed_by(&leaf_key, &self.0)
            .map_err(|e| eyre!("sign leaf cert: {e}"))?;
        Ok((leaf.pem(), leaf_key.serialize_pem()))
    }
}
