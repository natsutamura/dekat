//! TLS untuk transport peer-to-peer.
//!
//! Rancangan (M1):
//! 1. **Enkripsi** — TLS 1.3 (rustls) dengan sertifikat self-signed per instalasi.
//!    Sertifikat ini *bukan* dasar kepercayaan: ia hanya menyediakan kunci sesi.
//!    Karena tidak ada CA di jaringan lokal, verifikator menerima sertifikat apa pun.
//! 2. **Autentikasi identitas** — setelah handshake, kedua sisi menghitung
//!    *TLS exporter* (`export_keying_material`) yang hanya diketahui kedua ujung.
//!    Lalu masing-masing menandatangani `HELLO_CTX || exporter || id_pengirim || id_peer`
//!    dengan kunci Ed25519 miliknya (kunci yang sama yang menghasilkan node id).
//! 3. **Kenapa begitu** — tanpa exporter, penyerang bisa menjadi relay: meneruskan
//!    tanda tangan asli dari A ke B dan sebaliknya. Dengan exporter, tanda tangan A
//!    terikat pada sesi TLS A↔MITM, sehingga tidak sah di sesi MITM↔B. Relay gagal.
//!
//! Yang **tidak** dilindungi: penyerang yang menguasai jaringan saat pertama kali
//! dua node bertemu tetap bisa menyamar sebagai node itu (tidak ada PKI). Itu batas
//! wajar untuk aplikasi LAN tanpa server; dicatat di docs/PROTOCOL.md.

use anyhow::{anyhow, Context, Result};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime},
    ClientConfig, DigitallySignedStruct, ServerConfig, SignatureScheme,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_rustls::{
    rustls::{ClientConnection, ServerConnection},
    TlsAcceptor, TlsConnector,
};

/// Label exporter. Nilai ini ikut jadi bagian dari material kunci TLS,
/// jadi mengubahnya = memutus kompatibilitas.
const EXPORTER_LABEL: &[u8] = b"EXPORTER-lanchat-channel-v1";
const EXPORTER_LEN: usize = 32;

/// Konteks tanda tangan hello.
pub const HELLO_CTX: &[u8] = b"lanchat-hello-v1";

/// Konfigurasi TLS siap pakai untuk satu node.
#[derive(Clone)]
pub struct Tls {
    pub acceptor: TlsAcceptor,
    pub connector: TlsConnector,
    /// SHA-256 fingerprint sertifikat sendiri (hex) — untuk log/diagnostik.
    pub cert_fp: String,
}

/// Muat sertifikat+key dari `<dir>/tls/`, buat sekali kalau belum ada.
fn load_or_create_cert(dir: &Path) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>, String)> {
    let tls_dir = dir.join("tls");
    fs::create_dir_all(&tls_dir).with_context(|| format!("buat {}", tls_dir.display()))?;
    let cert_path = tls_dir.join("cert.pem");
    let key_path = tls_dir.join("key.pem");

    if !(cert_path.exists() && key_path.exists()) {
        let cert = rcgen::generate_simple_self_signed(vec!["lanchat.local".to_string()])
            .map_err(|e| anyhow!("gagal buat sertifikat: {e}"))?;
        fs::write(&cert_path, cert.cert.pem())?;
        fs::write(&key_path, cert.signing_key.serialize_pem())?;
        restrict_permissions(&key_path);
    }

    let cert_pem = fs::read(&cert_path)?;
    let key_pem = fs::read(&key_path)?;
    let certs = rustls_pemfile::certs(&mut cert_pem.as_slice())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("parse cert.pem")?;
    let key = rustls_pemfile::private_key(&mut key_pem.as_slice())
        .context("parse key.pem")?
        .ok_or_else(|| anyhow!("key.pem tidak berisi private key"))?;
    let cert = certs.into_iter().next().ok_or_else(|| anyhow!("cert.pem kosong"))?;

    let fp = sha256_hex(cert.as_ref());
    Ok((cert, key, fp))
}

#[cfg(unix)]
fn restrict_permissions(path: &PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &PathBuf) {}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

/// Verifikator yang menerima sertifikat apa pun.
///
/// Ini **disengaja**: kepercayaan tidak berasal dari sertifikat, melainkan dari
/// tanda tangan Ed25519 di atas TLS exporter (lihat modul ini). Menerima sertifikat
/// sembarang aman selama langkah itu wajib dan diverifikasi.
#[derive(Debug)]
struct AcceptAnyCert(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

impl Tls {
    /// Siapkan TLS untuk node yang datanya ada di `dir`.
    pub fn init(dir: &Path) -> Result<Self> {
        // ring dipilih eksplisit supaya build tidak butuh toolchain C tambahan.
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let (cert, key, cert_fp) = load_or_create_cert(dir)?;

        let server = ServerConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|e| anyhow!("versi TLS: {e}"))?
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .context("pasang sertifikat server")?;

        let client = ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|e| anyhow!("versi TLS: {e}"))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyCert(provider)))
            .with_no_client_auth();

        Ok(Self {
            acceptor: TlsAcceptor::from(Arc::new(server)),
            connector: TlsConnector::from(Arc::new(client)),
            cert_fp,
        })
    }

    /// Bungkus koneksi TCP keluar dengan TLS.
    pub async fn connect(&self, stream: tokio::net::TcpStream) -> Result<tokio_rustls::client::TlsStream<tokio::net::TcpStream>> {
        // SNI apa pun boleh: sertifikat tidak dipakai untuk memutuskan kepercayaan.
        let name = ServerName::try_from("lanchat.local").map_err(|e| anyhow!("server name: {e}"))?;
        self.connector
            .connect(name, stream)
            .await
            .context("handshake TLS keluar")
    }

    /// Bungkus koneksi TCP masuk dengan TLS.
    pub async fn accept(&self, stream: tokio::net::TcpStream) -> Result<tokio_rustls::server::TlsStream<tokio::net::TcpStream>> {
        self.acceptor
            .accept(stream)
            .await
            .context("handshake TLS masuk")
    }
}

/// Hitung nilai channel binding dari sesi TLS.
///
/// Kedua ujung mendapat nilai identik; penyadap/relay tidak bisa menirunya karena
/// nilainya diturunkan dari rahasia sesi TLS.
pub fn exporter_client(conn: &ClientConnection) -> Result<[u8; EXPORTER_LEN]> {
    let mut out = [0u8; EXPORTER_LEN];
    conn.export_keying_material(&mut out, EXPORTER_LABEL, Some(&[]))
        .map_err(|e| anyhow!("exporter TLS: {e}"))?;
    Ok(out)
}

pub fn exporter_server(conn: &ServerConnection) -> Result<[u8; EXPORTER_LEN]> {
    let mut out = [0u8; EXPORTER_LEN];
    conn.export_keying_material(&mut out, EXPORTER_LABEL, Some(&[]))
        .map_err(|e| anyhow!("exporter TLS: {e}"))?;
    Ok(out)
}

/// Pesan yang ditandatangani saat hello.
///
/// `a` = pengirim, `b` = penerima. Urutan tidak boleh membingungkan kedua sisi,
/// jadi selalu diurutkan: `min || max`, bukan "pengirim dulu".
pub fn hello_msg(exporter: &[u8], a: &str, b: &str) -> Vec<u8> {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let mut m = Vec::with_capacity(HELLO_CTX.len() + EXPORTER_LEN + lo.len() + hi.len() + 2);
    m.extend_from_slice(HELLO_CTX);
    m.extend_from_slice(exporter);
    m.push(0);
    m.extend_from_slice(lo.as_bytes());
    m.push(0);
    m.extend_from_slice(hi.as_bytes());
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_msg_simetris() {
        let ex = [7u8; EXPORTER_LEN];
        assert_eq!(hello_msg(&ex, "aaa", "bbb"), hello_msg(&ex, "bbb", "aaa"));
        assert_ne!(hello_msg(&ex, "aaa", "bbb"), hello_msg(&[8u8; EXPORTER_LEN], "aaa", "bbb"));
    }

    #[test]
    fn sertifikat_dibuat_dan_dimuat_ulang() {
        let dir = std::env::temp_dir().join(format!("lanchat-tls-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let a = Tls::init(&dir).unwrap();
        let b = Tls::init(&dir).unwrap();
        // fingerprint harus sama: sertifikat tidak dibuat ulang
        assert_eq!(a.cert_fp, b.cert_fp);
        assert_eq!(a.cert_fp.len(), 64);
        let _ = fs::remove_dir_all(&dir);
    }
}
