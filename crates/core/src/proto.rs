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
/// Ukuran potongan file mentah (base64 ~1.37x lebih besar, aman di bawah MAX_FRAME).
pub const FILE_CHUNK_BYTES: usize = 128 * 1024;
/// Batas ukuran file yang boleh dikirim (biar tidak mengunci memori/disk).
pub const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

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
        /// Tanda tangan Ed25519 (hex) atas `tls::hello_msg`.
        ///
        /// Default kosong = peer lama (pra-TLS). Koneksi tanpa tanda tangan
        /// hanya diterima di mode kompatibilitas, dan dianggap "belum terverifikasi".
        #[serde(default, skip_serializing_if = "String::is_empty")]
        sig: String,
        /// Public key Ed25519 lengkap (hex 32 byte). Wajib ada kalau `sig` ada;
        /// `id` harus sama dengan 8 byte pertamanya. Dikirim eksplisit supaya
        /// penerima bisa memverifikasi tanda tangan tanpa menebak dari `id`.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pk: String,
    },
    /// Bukti identitas: tanda tangan Ed25519 atas sesi TLS ini.
    ///
    /// Dikirim kedua arah setelah `Hello`, karena tanda tangan mengikat
    /// pasangan id — dan id peer baru diketahui setelah `Hello` diterima.
    /// Profil publik: nama tampilan + avatar (PNG base64, boleh kosong).
    ///
    /// Dikirim setelah `Auth` (saat koneksi baru) dan setiap kali user mengubah
    /// profilnya, supaya peer lain langsung memakai nama/foto terbaru.
    Profile {
        id: String,
        name: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        avatar: String,
        ts: u64,
    },
    Auth {
        /// Public key Ed25519 lengkap (hex 32 byte).
        pk: String,
        /// Tanda tangan hex atas `tls::hello_msg`.
        sig: String,
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
        /// Nama grup. Tanpa ini penerima hanya melihat label generik "Grup",
        /// karena room id deterministik tidak menyimpan nama apa pun.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        room_name: String,
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
    /// Tawaran kirim file. `chunks` = jumlah potongan, `size` = byte total.
    FileOffer {
        id: String,
        from: String,
        from_name: String,
        to: String,
        name: String,
        size: u64,
        sha256: String,
        chunks: u32,
        ts: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        members: Option<Vec<String>>,
    },
    /// Potongan file (base64). Dikirim berurutan setelah offer diterima.
    FileChunk {
        id: String,
        idx: u32,
        data: String,
    },
    /// Konfirmasi atas FileOffer: `accept=true` siap terima, `false` = tolak.
    FileAccept {
        id: String,
        accept: bool,
        #[serde(default)]
        reason: String,
        /// Byte yang sudah dimiliki penerima (untuk resume). 0 = mulai dari awal.
        #[serde(default)]
        have: u64,
    },
    /// Terima kasih: file utuh & terverifikasi.
    FileDone {
        id: String,
        ok: bool,
        #[serde(default)]
        reason: String,
    },
    /// Ajakan voice call. `call_id` dipakai untuk semua frame call berikutnya.
    CallOffer {
        call_id: String,
        from: String,
        from_name: String,
        to: String,
        /// SDP offer WebRTC (audio saja)
        sdp: String,
        ts: u64,
        /// Daftar peserta panggilan (termasuk pemanggil). Kosong = panggilan 1:1.
        ///
        /// Penerima memakai ini untuk membangun **mesh**: setelah menerima, ia
        /// memanggil peserta lain yang id-nya lebih besar, sehingga tiap pasangan
        /// punya tepat satu koneksi (aturan dial yang sama dengan transport utama).
        #[serde(default)]
        members: Vec<String>,
    },
    /// Jawaban atas [`Frame::CallOffer`]. `accept=false` → panggilan ditolak.
    CallAnswer {
        call_id: String,
        from: String,
        accept: bool,
        /// SDP answer; kosong kalau ditolak
        #[serde(default)]
        sdp: String,
        #[serde(default)]
        reason: String,
    },
    /// Kandidat ICE (trickle). Dikirim setelah offer/answer.
    CallIce {
        call_id: String,
        from: String,
        candidate: String,
        #[serde(default)]
        sdp_mid: String,
        #[serde(default)]
        sdp_mline_index: u32,
    },
    /// Panggilan diakhiri (salah satu sisi menutup).
    CallEnd {
        call_id: String,
        from: String,
        #[serde(default)]
        reason: String,
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

/// Nama file aman: buang komponen path & karakter kontrol, batasi 180 char.
/// Dipakai sebelum menyentuh filesystem — nama dari jaringan tidak dipercaya.
pub fn safe_filename(raw: &str) -> String {
    let base = raw
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("file")
        .trim()
        .trim_start_matches('.');
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control() && *c != ':' && *c != '*' && *c != '?' && *c != '"' && *c != '<' && *c != '>' && *c != '|')
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned.chars().take(180).collect()
    }
}

/// `1536` → `"1,5 KB"`. Buat ditampilkan di UI.
pub fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.1} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else {
        format!("{:.2} GB", b / (KB * KB * KB))
    }
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
            room_name: String::new(),
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
    fn safe_filename_strips_paths_and_traversal() {
        assert_eq!(safe_filename("../../etc/passwd"), "etc/passwd".rsplit('/').next().unwrap());
        assert_eq!(safe_filename("/etc/passwd"), "passwd");
        assert_eq!(safe_filename("C:\\Windows\\x.txt"), "x.txt");
        assert_eq!(safe_filename("...."), "file");
        assert_eq!(safe_filename(""), "file");
        assert_eq!(safe_filename("foto liburan.jpg"), "foto liburan.jpg");
        assert!(!safe_filename("a\u{0}b.txt").contains('\u{0}'));
        assert_eq!(safe_filename(&"x".repeat(400)).chars().count(), 180);
    }

    #[test]
    fn human_size_formats() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn file_offer_roundtrip() {
        let f = Frame::FileOffer {
            id: "f1".into(),
            from: "a".into(),
            from_name: "ray".into(),
            to: "b".into(),
            name: "foto.jpg".into(),
            size: 2048,
            sha256: "ab".into(),
            chunks: 2,
            ts: 1,
            members: None,
        };
        let j = serde_json::to_string(&f).unwrap();
        assert!(j.contains("\"t\":\"file_offer\""), "{j}");
        let back: Frame = serde_json::from_str(&j).unwrap();
        match back {
            Frame::FileOffer { size, chunks, .. } => {
                assert_eq!(size, 2048);
                assert_eq!(chunks, 2);
            }
            other => panic!("salah tipe: {other:?}"),
        }
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