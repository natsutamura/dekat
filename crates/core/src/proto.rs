//! Wire protocol M0.
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

pub const PROTO_VERSION: u8 = 1;
pub const MULTICAST_ADDR: &str = "239.255.42.99";
pub const DISCOVERY_PORT: u16 = 41234;
pub const DEFAULT_APP_PORT: u16 = 41235;
pub const ANNOUNCE_INTERVAL_SECS: u64 = 5;
pub const PEER_TTL_SECS: u64 = 16;
pub const MAX_FRAME: usize = 8 * 1024 * 1024;

/// Alamat tujuan khusus: broadcast ke semua peer yang terhubung.
pub const BROADCAST: &str = "*";

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
    },
    Text {
        id: String,
        from: String,
        from_name: String,
        /// `"*"` = broadcast, atau node id peer tujuan
        to: String,
        ts: u64,
        body: String,
    },
    Ack {
        id: String,
        ts: u64,
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

/// `HH:MM:SS` UTC — cukup buat log CLI, UI nanti pakai format lokal.
pub fn fmt_hms(ms: u64) -> String {
    let s = (ms / 1000) % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
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
        };
        let j = serde_json::to_string(&f).unwrap();
        assert!(j.contains("\"t\":\"text\""), "{j}");
        let back: Frame = serde_json::from_str(&j).unwrap();
        assert!(matches!(back, Frame::Text { .. }));
    }

    #[test]
    fn fmt_hms_is_utc() {
        assert_eq!(fmt_hms(0), "00:00:00");
        assert_eq!(fmt_hms(3_723_000), "01:02:03");
    }
}
