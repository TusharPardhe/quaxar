//! Rust port of `xrpl/basics/make_SSLContext.h`.

use openssl::asn1::{Asn1Integer, Asn1Time};
use openssl::bn::{BigNum, MsbOption};
use openssl::dh::Dh;
use openssl::error::ErrorStack;
use openssl::hash::MessageDigest;
use openssl::pkey::{PKey, Private};
use openssl::rsa::Rsa;
use openssl::ssl::{
    SslContext as OpenSslContext, SslContextBuilder, SslFiletype, SslMethod, SslOptions,
    SslVerifyMode as OpenSslVerifyMode,
};
use openssl::x509::extension::{
    BasicConstraints, ExtendedKeyUsage, KeyUsage, SubjectKeyIdentifier,
};
use openssl::x509::{X509, X509NameBuilder};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use time::{OffsetDateTime, Time};

pub const DEFAULT_CIPHER_LIST: &str = "TLSv1.2:!CBC:!DSS:!PSK:!eNULL:!aNULL";
const DEFAULT_RSA_KEY_BITS: u32 = 2048;
const CERT_VALIDITY_SECONDS: i64 = 2 * 365 * 24 * 60 * 60;
const DEFAULT_DH_PEM: &str = "-----BEGIN DH PARAMETERS-----\n\
MIIBCAKCAQEApKSWfR7LKy0VoZ/SDCObCvJ5HKX2J93RJ+QN8kJwHh+uuA8G+t8Q\n\
MDRjL5HanlV/sKN9HXqBc7eqHmmbqYwIXKUt9MUZTLNheguddxVlc2IjdP5i9Ps8\n\
l7su8tnP0l1JvC6Rfv3epRsEAw/ZW/lC2IwkQPpOmvnENQhQ6TgrUzcGkv4Bn0X6\n\
pxrDSBpZ+45oehGCUAtcbY8b02vu8zPFoxqo6V/+MIszGzldlik5bVqrJpVF6E8C\n\
tRqHjj6KuDbPbjc+pRGvwx/BSO3SULxmYu9J1NOk090MU1CMt6IJY7TpEc9Xrac9\n\
9yqY3xXZID240RRcaJ25+U4lszFPqP+CEwIBAg==\n\
-----END DH PARAMETERS-----";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SslContextMode {
    Anonymous,
    Authenticated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SslVerifyMode {
    None,
    Peer,
}

#[derive(Clone)]
pub struct SslContext {
    inner: Arc<OpenSslContext>,
    cipher_list: String,
    mode: SslContextMode,
    verify_mode: SslVerifyMode,
    key_file: Option<PathBuf>,
    cert_file: Option<PathBuf>,
    chain_file: Option<PathBuf>,
}

impl fmt::Debug for SslContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SslContext")
            .field("cipher_list", &self.cipher_list)
            .field("mode", &self.mode)
            .field("verify_mode", &self.verify_mode)
            .field("key_file", &self.key_file)
            .field("cert_file", &self.cert_file)
            .field("chain_file", &self.chain_file)
            .finish()
    }
}

impl SslContext {
    pub fn cipher_list(&self) -> &str {
        &self.cipher_list
    }

    pub fn mode(&self) -> SslContextMode {
        self.mode
    }

    pub fn verify_mode(&self) -> SslVerifyMode {
        self.verify_mode
    }

    pub fn key_file(&self) -> Option<&Path> {
        self.key_file.as_deref()
    }

    pub fn cert_file(&self) -> Option<&Path> {
        self.cert_file.as_deref()
    }

    pub fn chain_file(&self) -> Option<&Path> {
        self.chain_file.as_deref()
    }

    pub fn inner(&self) -> &OpenSslContext {
        self.inner.as_ref()
    }
}

pub type SharedSslContext = Arc<SslContext>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsIdentityDer {
    certificate_der: Vec<u8>,
    extra_chain_der: Vec<Vec<u8>>,
    private_key_pkcs8_der: Vec<u8>,
}

impl TlsIdentityDer {
    pub fn certificate_der(&self) -> &[u8] {
        &self.certificate_der
    }

    pub fn extra_chain_der(&self) -> &[Vec<u8>] {
        &self.extra_chain_der
    }

    pub fn private_key_pkcs8_der(&self) -> &[u8] {
        &self.private_key_pkcs8_der
    }

    pub fn certificate_chain_der(&self) -> Vec<Vec<u8>> {
        let mut chain = Vec::with_capacity(1 + self.extra_chain_der.len());
        chain.push(self.certificate_der.clone());
        chain.extend(self.extra_chain_der.clone());
        chain
    }
}

#[derive(Debug)]
pub enum SslContextError {
    MissingAuthenticationMaterial,
    MissingPrivateKeyMaterial,
    MissingFile(PathBuf),
    Io(std::io::Error),
    OpenSsl(ErrorStack),
    Initialization(String),
}

impl fmt::Display for SslContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingAuthenticationMaterial => {
                f.write_str("authenticated SSL context requires certificate or chain material")
            }
            Self::MissingPrivateKeyMaterial => {
                f.write_str("authenticated TLS identity requires a private key file")
            }
            Self::MissingFile(path) => write!(f, "missing SSL file: {}", path.display()),
            Self::Io(error) => write!(f, "SSL I/O error: {error}"),
            Self::OpenSsl(error) => write!(f, "OpenSSL error: {error}"),
            Self::Initialization(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SslContextError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::OpenSsl(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for SslContextError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ErrorStack> for SslContextError {
    fn from(error: ErrorStack) -> Self {
        Self::OpenSsl(error)
    }
}

#[derive(Clone)]
struct AnonymousPemMaterial {
    cert_pem: Vec<u8>,
    key_pem: Vec<u8>,
}

static ANONYMOUS_MATERIAL: OnceLock<Result<AnonymousPemMaterial, String>> = OnceLock::new();

pub fn anonymous_tls_identity_der() -> Result<TlsIdentityDer, SslContextError> {
    let material = anonymous_material()?;
    let cert = X509::from_pem(&material.cert_pem)?;
    let key = PKey::private_key_from_pem(&material.key_pem)?;
    Ok(TlsIdentityDer {
        certificate_der: cert.to_der()?,
        extra_chain_der: Vec::new(),
        private_key_pkcs8_der: key.private_key_to_pkcs8()?,
    })
}

pub fn authenticated_tls_identity_der(
    key_file: impl AsRef<Path>,
    cert_file: impl AsRef<Path>,
    chain_file: impl AsRef<Path>,
) -> Result<TlsIdentityDer, SslContextError> {
    let key_file = path_from_input(key_file.as_ref());
    let cert_file = path_from_input(cert_file.as_ref());
    let chain_file = path_from_input(chain_file.as_ref());

    if cert_file.is_none() && chain_file.is_none() {
        return Err(SslContextError::MissingAuthenticationMaterial);
    }
    let Some(key_file) = key_file.as_ref() else {
        return Err(SslContextError::MissingPrivateKeyMaterial);
    };

    for path in [Some(key_file), cert_file.as_ref(), chain_file.as_ref()]
        .into_iter()
        .flatten()
    {
        if !path.is_file() {
            return Err(SslContextError::MissingFile(path.clone()));
        }
    }

    let key = PKey::private_key_from_pem(&fs::read(key_file)?)?;
    let mut certificate_der = cert_file
        .as_ref()
        .map(|path| -> Result<Vec<u8>, SslContextError> {
            let pem = fs::read(path)?;
            let cert = X509::from_pem(&pem)?;
            Ok(cert.to_der()?)
        })
        .transpose()?;
    let mut extra_chain_der = Vec::new();

    if let Some(chain_file) = chain_file.as_ref() {
        let mut chain = X509::stack_from_pem(&fs::read(chain_file)?)?;
        if certificate_der.is_none() && !chain.is_empty() {
            certificate_der = Some(chain.remove(0).to_der()?);
        }
        for cert in chain {
            extra_chain_der.push(cert.to_der()?);
        }
    }

    let Some(certificate_der) = certificate_der else {
        return Err(SslContextError::MissingAuthenticationMaterial);
    };

    Ok(TlsIdentityDer {
        certificate_der,
        extra_chain_der,
        private_key_pkcs8_der: key.private_key_to_pkcs8()?,
    })
}

pub fn make_ssl_context(cipher_list: impl AsRef<str>) -> Result<SharedSslContext, SslContextError> {
    let cipher_list = normalize_cipher_list(cipher_list.as_ref());
    let mut builder = build_context(&cipher_list)?;
    init_anonymous(&mut builder)?;
    builder.set_verify(OpenSslVerifyMode::NONE);

    Ok(Arc::new(SslContext {
        inner: Arc::new(builder.build()),
        cipher_list,
        mode: SslContextMode::Anonymous,
        verify_mode: SslVerifyMode::None,
        key_file: None,
        cert_file: None,
        chain_file: None,
    }))
}

pub fn make_ssl_context_authed(
    key_file: impl AsRef<Path>,
    cert_file: impl AsRef<Path>,
    chain_file: impl AsRef<Path>,
    cipher_list: impl AsRef<str>,
) -> Result<SharedSslContext, SslContextError> {
    let key_file = path_from_input(key_file.as_ref());
    let cert_file = path_from_input(cert_file.as_ref());
    let chain_file = path_from_input(chain_file.as_ref());

    if cert_file.is_none() && chain_file.is_none() {
        return Err(SslContextError::MissingAuthenticationMaterial);
    }

    for path in [&key_file, &cert_file, &chain_file].into_iter().flatten() {
        if !path.is_file() {
            return Err(SslContextError::MissingFile(path.clone()));
        }
    }

    let cipher_list = normalize_cipher_list(cipher_list.as_ref());
    let mut builder = build_context(&cipher_list)?;
    init_authenticated(
        &mut builder,
        key_file.as_deref(),
        cert_file.as_deref(),
        chain_file.as_deref(),
    )?;

    Ok(Arc::new(SslContext {
        inner: Arc::new(builder.build()),
        cipher_list,
        mode: SslContextMode::Authenticated,
        verify_mode: SslVerifyMode::None,
        key_file,
        cert_file,
        chain_file,
    }))
}

#[allow(non_snake_case)]
pub fn make_SSLContext(cipher_list: impl AsRef<str>) -> Result<SharedSslContext, SslContextError> {
    make_ssl_context(cipher_list)
}

#[allow(non_snake_case)]
pub fn make_SSLContextAuthed(
    key_file: impl AsRef<Path>,
    cert_file: impl AsRef<Path>,
    chain_file: impl AsRef<Path>,
    cipher_list: impl AsRef<str>,
) -> Result<SharedSslContext, SslContextError> {
    make_ssl_context_authed(key_file, cert_file, chain_file, cipher_list)
}

fn build_context(cipher_list: &str) -> Result<SslContextBuilder, SslContextError> {
    let mut builder = SslContextBuilder::new(SslMethod::tls())?;
    builder.set_options(
        SslOptions::ALL
            | SslOptions::NO_SSLV2
            | SslOptions::NO_SSLV3
            | SslOptions::NO_TLSV1
            | SslOptions::NO_TLSV1_1
            | SslOptions::SINGLE_DH_USE
            | SslOptions::NO_COMPRESSION
            | SslOptions::NO_RENEGOTIATION,
    );
    builder.set_cipher_list(cipher_list)?;
    let dh = Dh::params_from_pem(DEFAULT_DH_PEM.as_bytes())?;
    builder.set_tmp_dh(&dh)?;
    Ok(builder)
}

fn init_anonymous(builder: &mut SslContextBuilder) -> Result<(), SslContextError> {
    let material = anonymous_material()?;
    let cert = X509::from_pem(&material.cert_pem)?;
    let key = PKey::private_key_from_pem(&material.key_pem)?;
    builder.set_certificate(&cert)?;
    builder.set_private_key(&key)?;
    Ok(())
}

fn init_authenticated(
    builder: &mut SslContextBuilder,
    key_file: Option<&Path>,
    cert_file: Option<&Path>,
    chain_file: Option<&Path>,
) -> Result<(), SslContextError> {
    let mut cert_set = false;

    if let Some(cert_file) = cert_file {
        builder.set_certificate_file(cert_file, SslFiletype::PEM)?;
        cert_set = true;
    }

    if let Some(chain_file) = chain_file {
        let chain_pem = fs::read(chain_file)?;
        let mut chain = X509::stack_from_pem(&chain_pem)?;
        if !cert_set && !chain.is_empty() {
            let cert = chain.remove(0);
            builder.set_certificate(&cert)?;
        }
        for cert in chain {
            builder.add_extra_chain_cert(cert)?;
        }
    }

    if let Some(key_file) = key_file {
        builder.set_private_key_file(key_file, SslFiletype::PEM)?;
    }

    builder.check_private_key()?;
    Ok(())
}

fn anonymous_material() -> Result<AnonymousPemMaterial, SslContextError> {
    ANONYMOUS_MATERIAL
        .get_or_init(|| create_anonymous_material().map_err(|error| error.to_string()))
        .clone()
        .map_err(SslContextError::Initialization)
}

fn create_anonymous_material() -> Result<AnonymousPemMaterial, SslContextError> {
    let rsa = Rsa::generate(DEFAULT_RSA_KEY_BITS)?;
    let key = PKey::from_rsa(rsa)?;
    let cert = build_anonymous_certificate(&key)?;
    Ok(AnonymousPemMaterial {
        cert_pem: cert.to_pem()?,
        key_pem: key.private_key_to_pem_pkcs8()?,
    })
}

fn build_anonymous_certificate(key: &PKey<Private>) -> Result<X509, SslContextError> {
    let mut builder = X509::builder()?;
    builder.set_version(2)?;

    let validity_start = anonymous_validity_start()?;
    let validity_end =
        Asn1Time::from_unix(validity_start.unix_timestamp() + CERT_VALIDITY_SECONDS)?;
    let not_before = Asn1Time::from_unix(validity_start.unix_timestamp())?;
    builder.set_not_before(&not_before)?;
    builder.set_not_after(&validity_end)?;

    let mut serial = BigNum::new()?;
    serial.rand(128, MsbOption::MAYBE_ZERO, false)?;
    let serial = Asn1Integer::from_bn(&serial)?;
    builder.set_serial_number(&serial)?;

    let name = X509NameBuilder::new()?.build();
    builder.set_subject_name(&name)?;
    builder.set_issuer_name(&name)?;
    builder.set_pubkey(key)?;

    builder.append_extension(BasicConstraints::new().critical().build()?)?;
    builder.append_extension(
        ExtendedKeyUsage::new()
            .critical()
            .server_auth()
            .client_auth()
            .build()?,
    )?;
    builder.append_extension(KeyUsage::new().critical().digital_signature().build()?)?;
    let subject_key_identifier =
        SubjectKeyIdentifier::new().build(&builder.x509v3_context(None, None))?;
    builder.append_extension(subject_key_identifier)?;

    builder.sign(key, MessageDigest::sha256())?;
    Ok(builder.build())
}

fn anonymous_validity_start() -> Result<OffsetDateTime, SslContextError> {
    let ts = OffsetDateTime::now_utc().unix_timestamp() - (25 * 60 * 60);
    let rounded = OffsetDateTime::from_unix_timestamp(ts)
        .map_err(|error| SslContextError::Initialization(error.to_string()))?
        .replace_time(Time::MIDNIGHT);
    Ok(rounded)
}

fn normalize_cipher_list(cipher_list: &str) -> String {
    if cipher_list.is_empty() {
        DEFAULT_CIPHER_LIST.to_owned()
    } else {
        cipher_list.to_owned()
    }
}

fn path_from_input(path: &Path) -> Option<PathBuf> {
    (!path.as_os_str().is_empty()).then(|| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_CIPHER_LIST, SslContextError, SslContextMode, SslVerifyMode,
        anonymous_tls_identity_der, authenticated_tls_identity_der, make_ssl_context,
        make_ssl_context_authed,
    };
    use openssl::asn1::Asn1Time;
    use openssl::hash::MessageDigest;
    use openssl::pkey::PKey;
    use openssl::rsa::Rsa;
    use openssl::x509::{X509, X509NameBuilder};
    use std::fs;
    use std::path::PathBuf;

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path).expect("temp dir");
        path
    }

    fn write_test_identity(root: &std::path::Path) -> (PathBuf, PathBuf, PathBuf) {
        let key = PKey::from_rsa(Rsa::generate(2048).expect("generate rsa")).expect("pkey");
        let mut cert = X509::builder().expect("builder");
        cert.set_version(2).expect("version");
        let mut name = X509NameBuilder::new().expect("name builder");
        name.append_entry_by_text("CN", "localhost").expect("cn");
        let name = name.build();
        cert.set_subject_name(&name).expect("subject");
        cert.set_issuer_name(&name).expect("issuer");
        cert.set_pubkey(&key).expect("pubkey");
        cert.set_not_before(&Asn1Time::days_from_now(0).expect("not before"))
            .expect("set not before");
        cert.set_not_after(&Asn1Time::days_from_now(30).expect("not after"))
            .expect("set not after");
        cert.sign(&key, MessageDigest::sha256()).expect("sign");
        let cert = cert.build();

        let cert_path = root.join("server.cert");
        let key_path = root.join("server.key");
        let chain_path = root.join("chain.pem");
        fs::write(&cert_path, cert.to_pem().expect("cert pem")).expect("write cert");
        fs::write(&key_path, key.private_key_to_pem_pkcs8().expect("key pem")).expect("write key");
        fs::write(&chain_path, cert.to_pem().expect("chain pem")).expect("write chain");
        (key_path, cert_path, chain_path)
    }

    #[test]
    fn anonymous_context_uses_default_cipher_list_and_verify_none() {
        let context = make_ssl_context("").expect("anonymous context");
        assert_eq!(context.cipher_list(), DEFAULT_CIPHER_LIST);
        assert_eq!(context.mode(), SslContextMode::Anonymous);
        assert_eq!(context.verify_mode(), SslVerifyMode::None);
        let _ = context.inner();

        let identity = anonymous_tls_identity_der().expect("anonymous identity");
        assert!(!identity.certificate_der().is_empty());
        assert!(!identity.private_key_pkcs8_der().is_empty());
    }

    #[test]
    fn authed_context_requires_present_files() {
        let root = unique_temp_dir("ssl-context");
        let (key, cert, chain) = write_test_identity(&root);

        let context =
            make_ssl_context_authed(&key, &cert, PathBuf::new(), "HIGH:!aNULL").expect("auth");
        assert_eq!(context.mode(), SslContextMode::Authenticated);
        assert_eq!(context.verify_mode(), SslVerifyMode::None);
        assert_eq!(context.cipher_list(), "HIGH:!aNULL");
        assert_eq!(context.key_file(), Some(key.as_path()));
        assert_eq!(context.cert_file(), Some(cert.as_path()));
        assert_eq!(context.chain_file(), None);

        let identity = authenticated_tls_identity_der(&key, &cert, &chain).expect("der identity");
        assert!(!identity.certificate_der().is_empty());
        assert_eq!(identity.extra_chain_der().len(), 1);
        assert!(!identity.private_key_pkcs8_der().is_empty());

        let chain_only = make_ssl_context_authed(PathBuf::new(), PathBuf::new(), &chain, "")
            .expect_err("chain without key should fail");
        assert!(matches!(chain_only, SslContextError::OpenSsl(_)));

        let missing_key_identity =
            authenticated_tls_identity_der(PathBuf::new(), &cert, &chain).expect_err("missing key");
        assert!(matches!(
            missing_key_identity,
            SslContextError::MissingPrivateKeyMaterial
        ));

        let error = make_ssl_context_authed(root.join("missing.key"), &cert, PathBuf::new(), "")
            .expect_err("missing key should fail");
        assert!(matches!(error, SslContextError::MissingFile(_)));

        let material_error =
            make_ssl_context_authed(PathBuf::new(), PathBuf::new(), PathBuf::new(), "")
                .expect_err("missing material");
        assert!(matches!(
            material_error,
            SslContextError::MissingAuthenticationMaterial
        ));

        let _ = fs::remove_dir_all(root);
    }
}
