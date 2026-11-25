//! Identitas node: keypair Ed25519 yang dipersist di `identity.json`.
//!
//! `id`      = 8 byte pertama public key (hex, 16 char) — dipakai sebagai node id.
//! `fp`      = `nama#xxxx` — fingerprint pendek yang ditampilkan ke user.
//!
//! M0 belum memakai signature di wire (menyusul di M1 bersama TLS/TOFU),
//! tapi keypair-nya sudah stabil supaya id tidak berubah antar restart.

use anyhow::{anyhow, Context, Result};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Serialize, Deserialize)]
struct Stored {
    name: Option<String>,
    secret_hex: String,
}

pub struct Identity {
    pub key: SigningKey,
    pub name: String,
}

impl Identity {
    pub fn id(&self) -> String {
        hex::encode(&self.key.verifying_key().to_bytes()[..8])
    }

    pub fn fp(&self) -> String {
        format!("{}#{}", self.name, &self.id()[..4])
    }

    /// Public key Ed25519 lengkap (hex 32 byte).
    pub fn pk_hex(&self) -> String {
        hex::encode(self.key.verifying_key().to_bytes())
    }

    /// Tanda tangani `msg`, hasil hex 64 byte.
    pub fn sign_hex(&self, msg: &[u8]) -> String {
        use ed25519_dalek::Signer;
        hex::encode(self.key.sign(msg).to_bytes())
    }

    /// Verifikasi tanda tangan hex atas `msg` dengan public key hex.
    ///
    /// `id` (kalau diberikan) harus sama dengan 8 byte pertama public key —
    /// ini yang mencegah peer mengaku ber-id lain dari kuncinya.
    pub fn verify_hex(pk_hex: &str, msg: &[u8], sig_hex: &str, id: Option<&str>) -> Result<()> {
        use ed25519_dalek::{Signature, VerifyingKey};
        let pk_bytes = hex::decode(pk_hex).context("pk bukan hex")?;
        let pk: [u8; 32] = pk_bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("public key harus 32 byte"))?;
        if let Some(id) = id {
            if hex::encode(&pk[..8]) != id {
                return Err(anyhow!("id tidak cocok dengan public key"));
            }
        }
        let sig_bytes = hex::decode(sig_hex).context("sig bukan hex")?;
        let sig_arr: [u8; 64] = sig_bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("signature harus 64 byte"))?;
        let vk = VerifyingKey::from_bytes(&pk).map_err(|e| anyhow!("public key tidak valid: {e}"))?;
        // verify_strict: menolak signature dengan komponen non-canonical.
        vk.verify_strict(msg, &Signature::from_bytes(&sig_arr))
            .map_err(|e| anyhow!("tanda tangan tidak sah: {e}"))
    }

    /// Muat keypair dari `<dir>/identity.json`, buat baru kalau belum ada.
    pub fn load_or_create(dir: &Path, name: &str) -> Result<Self> {
        fs::create_dir_all(dir).with_context(|| format!("gagal buat dir {}", dir.display()))?;
        let path = dir.join("identity.json");

        let key = if path.exists() {
            let raw = fs::read_to_string(&path)?;
            let st: Stored = serde_json::from_str(&raw).context("identity.json rusak")?;
            let bytes = hex::decode(st.secret_hex)?;
            let seed: [u8; 32] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| anyhow!("secret key harus 32 byte"))?;
            SigningKey::from_bytes(&seed)
        } else {
            use rand::RngCore;
            let mut seed = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut seed);
            let k = SigningKey::from_bytes(&seed);
            let st = Stored {
                name: Some(name.to_string()),
                secret_hex: hex::encode(seed),
            };
            fs::write(&path, serde_json::to_string_pretty(&st)?)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
            }
            k
        };

        Ok(Self {
            key,
            name: name.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_stable_across_reload() {
        let dir = std::env::temp_dir().join(format!("lanchat-id-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = Identity::load_or_create(&dir, "ray").unwrap();
        let b = Identity::load_or_create(&dir, "ray").unwrap();
        assert_eq!(a.id(), b.id());
        assert_eq!(a.id().len(), 16);
        assert!(a.fp().starts_with("ray#"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
