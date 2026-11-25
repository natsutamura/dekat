//! Wire protocol (v2).
//!
//! Dua lapisan:
//! 1. **Discovery** — paket [`Announce`] JSON di UDP 41234 (multicast 239.255.42.99,
//!    fallback limited-broadcast + seed manual).
//! 2. **Transport** — TCP, frame = `u32 BE length` + JSON [`Frame`].
//!
//! Semua field waktu memakai epoch milidetik UTC.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const PROTO_VERSION: u8 = 2;
pub const MULTICAST_ADDR: &str = "239.255.42.99";
pub const DISCOVERY_PORT: u16 = 41234;
pub const DEFAULT_APP_PORT: u16 = 41235;
pub const ANNOUNCE_INTERVAL_SECS: u64 = 5;
pub const PEER_TTL_SECS: u16 = 16;
pub const MAX_FRAME: usize = 8 * 1024 * 1024;

/// Alamat tujuan khusus: broadcast ke semua peer yang terhubung.
pub const BROADCAST: &str = "*";

/// Presence yang dikirim ke jaringan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Online,
    Idle,
    Dnd,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Online => "online",
            Status::Idle => "idle",
            Status::Dnd => "dnd",
        }
    }

    pub fn parse(s: &str) -> Status {
        match s {
            "idle" => Status::Idle,
            "dnd" => Status::Dnd,
            _ => Status::Online,
        }
    }
}

impl Default for Status {
    fn default() -> Self {
        Status::Online
    }
}

/// Paket presence. Dikirim tiap [`ANNOUNCE_INTERVAL_SECS`] detik.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Announce {
    pub v: u8,
    pub id: String,
    pub name: String,
    pub fp: String,
    pub os: String,
    /// port TCP tempat peer ini mendengarkan frame
    pub port: u16,
    #[serde(default)]
    pub status: Status,
    pub ts: u64,
}

/// Frame yang mengalir di koneksi TCP.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Frame {
    Hello {
        v: u8,
        id: String,
        name: String,
        fp: String,
        port: u16,
        /// OS peer. Default kosong biar kompatibel dengan peer versi lama
        /// yang belum mengirim field ini.
        #[serde(default)]
        os: String,
        #[serde(default)]
        status: Status,
    },
    Text {
        id: String,
        from: String,
        from_name: String,
        /// `"*"` = broadcast, `#xxxx` = group/room, atau node id peer tujuan
        to: String,
        ts: u64,
        body: String,
        /// diisi kalau pesan group, supaya penerima bisa verifikasi keanggotaan
        #[serde(default, skip_serializing_if = "Option::is_none")]
        members: Option<Vec<String>>,
    },
    Ack {
        id: String,
        ts: u64,
    },
    Typing {
        from: String,
        to: String,
        on: bool,
    },
    Read {
        from: String,
        to: String,
        up_to_ts: u64,
    },
    Presence {
        id: String,
        status: Status,
    },
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn new_msg_id() -> String {
    use rand::RngCore;
    let mut b = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

/// `HH:MM:SS` UTC — cukup buat log CLI, UI pakai format lokal.
pub fn fmt_hms(ms: u64) -> String {
    let s = (ms / 1000) % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

/// Id room deterministik dari daftar anggota (id diri sendiri wajib ikut).
/// Semua node dengan himpunan anggota sama akan menghitung id yang sama,
/// jadi tidak perlu protokol invite untuk M1.
pub fn room_id(members: &[String]) -> String {
    let mut v: Vec<&str> = members.iter().map(|s| s.as_str()).collect();
    v.sort_unstable();
    v.dedup();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a 64
    for b in v.join("|").as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    let folded = (hash ^ (hash >> 32)) as u32;
    format!("#{folded:08x}")
}

pub fn is_room(target: &str) -> bool {
    target.starts_with('#')
}

pub async fn write_frame<W>(w: &mut W, f: &Frame) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let body = serde_json::to_vec(f)?;
    if body.len() > MAX_FRAME {
        bail!("frame terlalu besar ({} byte)", body.len());
    }
    w.write_all(&(body.len() as u32).to_be_bytes()).await?;
    w.write_all(&body).await?;
    w.flush().await?;
    Ok(())
}

/// `Ok(None)` = peer menutup koneksi dengan rapi.
pub async fn read_frame<R>(r: &mut R) -> Result<Option<Frame>>
where
    R: AsyncRead + Unpin,
{
    let mut len = [0u8; 4];
    match r.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let n = u32::from_be_bytes(len) as usize;
    if n == 0 || n > MAX_FRAME {
        bail!("panjang frame tidak masuk akal: {n}");
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).await?;
    Ok(Some(serde_json::from_slice(&buf)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_json_shape_is_tagged() {
        let f = Frame::Text {
            id: "abc".into(),
            from: "a".into(),
            from_name: "ray".into(),
            to: BROADCAST.into(),
            ts: 1,
            body: "hai".into(),
            members: None,
        };
        let j = serde_json::to_string(&f).unwrap();
        assert!(j.contains("\"t\":\"text\""), "{j}");
        assert!(!j.contains("members"), "members kosong tidak dikirim: {j}");
        let back: Frame = serde_json::from_str(&j).unwrap();
        assert!(matches!(back, Frame::Text { .. }));
    }

    #[test]
    fn fmt_hms_is_utc() {
        assert_eq!(fmt_hms(0), "00:00:00");
        assert_eq!(fmt_hms(3_723_000), "01:02:03");
    }

    #[test]
    fn room_id_is_order_independent() {
        let a = vec!["aaa".to_string(), "bbb".to_string(), "ccc".to_string()];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(room_id(&a), room_id(&b));
        assert!(room_id(&a).starts_with('#'));
        assert_ne!(room_id(&a), room_id(&["aaa".into(), "bbb".into()]));
    }

    #[test]
    fn status_roundtrip() {
        for s in [Status::Online, Status::Idle, Status::Dnd] {
            assert_eq!(Status::parse(s.as_str()), s);
        }
        // announce lama (tanpa field status) tetap bisa dibaca
        let old = r#"{"v":2,"id":"x","name":"n","fp":"f","os":"linux","port":1,"ts":0}"#;
        let a: Announce = serde_json::from_str(old).unwrap();
        assert_eq!(a.status, Status::Online);
    }
}
