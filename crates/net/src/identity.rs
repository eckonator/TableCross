//! The device identity: a key pair with a self-signed certificate, created on first start.
//!
//! Peers recognise each other by the SHA-256 fingerprint of this certificate (pinning), so the files
//! must never be replaced silently: a new identity means pairing again with every peer.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

const KEY_FILE: &str = "device.key";
const CERT_FILE: &str = "device.crt";
/// Name inside the certificate. Peers do not check it; they compare fingerprints.
pub(crate) const CERT_NAME: &str = "tablecross";

/// SHA-256 of a device certificate.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Fingerprint(pub [u8; 32]);

impl Fingerprint {
    pub fn of(cert_der: &[u8]) -> Self {
        Self(Sha256::digest(cert_der).into())
    }

    /// First 8 hex digits, enough to tell devices apart in lists and logs.
    pub fn short(&self) -> String {
        self.to_string()[..8].to_string()
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({})", self.short())
    }
}

impl FromStr for Fingerprint {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.len() != 64 || !s.is_ascii() {
            return Err("fingerprint must be 64 hex digits");
        }
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|_| "fingerprint must be 64 hex digits")?;
        }
        Ok(Self(out))
    }
}

impl Serialize for Fingerprint {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Fingerprint {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("cannot access the device identity in {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error(
        "the device identity in {0} is damaged; delete device.key and device.crt to create a new one \
         (all peers must be paired again)"
    )]
    Damaged(PathBuf),
    #[error("cannot create a certificate: {0}")]
    Generate(#[from] rcgen::Error),
}

/// Certificate and private key of this device.
pub struct Identity {
    cert: CertificateDer<'static>,
    key: PrivatePkcs8KeyDer<'static>,
    fingerprint: Fingerprint,
}

impl Clone for Identity {
    fn clone(&self) -> Self {
        Self {
            cert: self.cert.clone(),
            key: self.key.clone_key(),
            fingerprint: self.fingerprint,
        }
    }
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity")
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

impl Identity {
    /// Creates a fresh identity in memory (tests, or before saving).
    pub fn generate() -> Result<Self, IdentityError> {
        let certified = rcgen::generate_simple_self_signed(vec![CERT_NAME.to_string()])?;
        let cert = CertificateDer::from(certified.cert.der().to_vec());
        let key = PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der());
        Ok(Self::from_parts(cert, key))
    }

    /// Loads the identity from `dir`, or creates and saves a new one if there is none yet.
    pub fn load_or_create(dir: &Path) -> Result<Self, IdentityError> {
        let key_path = dir.join(KEY_FILE);
        let cert_path = dir.join(CERT_FILE);
        let io_err = |path: &Path| {
            let path = path.to_path_buf();
            move |source| IdentityError::Io { path, source }
        };
        if key_path.exists() || cert_path.exists() {
            let key = fs::read(&key_path).map_err(io_err(&key_path))?;
            let cert = fs::read(&cert_path).map_err(io_err(&cert_path))?;
            let identity = Self::from_parts(CertificateDer::from(cert), PrivatePkcs8KeyDer::from(key));
            if !identity.is_valid() {
                return Err(IdentityError::Damaged(dir.to_path_buf()));
            }
            return Ok(identity);
        }
        let identity = Self::generate()?;
        fs::create_dir_all(dir).map_err(io_err(dir))?;
        write_private(&key_path, identity.key.secret_pkcs8_der()).map_err(io_err(&key_path))?;
        fs::write(&cert_path, identity.cert.as_ref()).map_err(io_err(&cert_path))?;
        Ok(identity)
    }

    fn from_parts(cert: CertificateDer<'static>, key: PrivatePkcs8KeyDer<'static>) -> Self {
        let fingerprint = Fingerprint::of(&cert);
        Self { cert, key, fingerprint }
    }

    /// Key and certificate parse and belong together.
    fn is_valid(&self) -> bool {
        let provider = rustls::crypto::ring::default_provider();
        let Ok(key) = provider.key_provider.load_private_key(self.private_key()) else {
            return false;
        };
        rustls::sign::CertifiedKey::new(vec![self.cert.clone()], key)
            .keys_match()
            .is_ok()
    }

    pub fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    pub(crate) fn cert(&self) -> CertificateDer<'static> {
        self.cert.clone()
    }

    pub(crate) fn private_key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(self.key.clone_key())
    }
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    // The settings folder below the user profile is already private to the user on Windows
    fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_created_once_and_reloaded() {
        let dir = crate::test_dir("identity");
        let first = Identity::load_or_create(&dir).unwrap();
        let second = Identity::load_or_create(&dir).unwrap();
        assert_eq!(first.fingerprint(), second.fingerprint());
        assert_ne!(first.fingerprint(), Identity::generate().unwrap().fingerprint());
    }

    #[test]
    fn damaged_identity_is_reported_not_replaced() {
        let dir = crate::test_dir("identity-damaged");
        let original = Identity::load_or_create(&dir).unwrap();
        fs::write(dir.join(KEY_FILE), b"garbage").unwrap();
        assert!(matches!(Identity::load_or_create(&dir), Err(IdentityError::Damaged(_))));
        // A key from another identity does not match the certificate either
        let other = Identity::generate().unwrap();
        fs::write(dir.join(KEY_FILE), other.key.secret_pkcs8_der()).unwrap();
        assert!(matches!(Identity::load_or_create(&dir), Err(IdentityError::Damaged(_))));
        assert_eq!(fs::read(dir.join(CERT_FILE)).unwrap(), original.cert.as_ref());
    }

    #[test]
    fn fingerprint_text_round_trip() {
        let fp = Identity::generate().unwrap().fingerprint();
        assert_eq!(fp.to_string().parse::<Fingerprint>(), Ok(fp));
        assert_eq!(fp.short().len(), 8);
        assert!("xyz".parse::<Fingerprint>().is_err());
        assert!("zz".repeat(32).parse::<Fingerprint>().is_err());
        assert!("ä".repeat(32).parse::<Fingerprint>().is_err());
    }
}
