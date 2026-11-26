//! Penyimpanan lokal (SQLite): pesan, thread, setting.
//!
//! Satu file DB per profil (`<data_dir>/lanchat.db`). UI tidak pernah menyentuh SQL
//! langsung — semua lewat API ini supaya versi schema gampang dimigrasi.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Mutex};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS messages (
    id         TEXT PRIMARY KEY,
    thread_id  TEXT NOT NULL,
    from_id    TEXT NOT NULL,
    from_name  TEXT NOT NULL,
    body       TEXT NOT NULL,
    ts         INTEGER NOT NULL,
    mine       INTEGER NOT NULL DEFAULT 0,
    delivered  INTEGER NOT NULL DEFAULT 0,
    read       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS messages_thread_ts ON messages(thread_id, ts);

CREATE TABLE IF NOT EXISTS threads (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL,
    name          TEXT NOT NULL,
    members       TEXT NOT NULL DEFAULT '[]',
    last_ts       INTEGER NOT NULL DEFAULT 0,
    unread        INTEGER NOT NULL DEFAULT 0,
    manual_unread INTEGER NOT NULL DEFAULT 0,
    preview       TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS settings (
    k TEXT PRIMARY KEY,
    v TEXT NOT NULL
);

-- Lampiran per pesan. Satu pesan = maksimal satu file (cukup untuk chat).
CREATE TABLE IF NOT EXISTS attachments (
    msg_id    TEXT PRIMARY KEY,
    file_name TEXT NOT NULL,
    file_size INTEGER NOT NULL,
    file_path TEXT NOT NULL,
    sha256    TEXT NOT NULL,
    mime      TEXT NOT NULL DEFAULT ''
);
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessage {
    pub id: String,
    pub thread_id: String,
    pub from_id: String,
    pub from_name: String,
    pub body: String,
    pub ts: u64,
    pub mine: bool,
    pub delivered: bool,
    /// Sudah dibaca peer (read receipt). Terpisah dari `delivered`.
    #[serde(default)]
    pub read: bool,
    /// Lampiran (kalau ada). Tidak disimpan di tabel messages, di-join dari `attachments`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<Attachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Attachment {
    pub name: String,
    pub size: u64,
    /// path absolut di perangkat ini
    pub path: String,
    pub sha256: String,
    /// kosong untuk transfer antar-perangkat (kita tidak menebak mime)
    #[serde(default)]
    pub mime: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub id: String,
    /// `"dm"` atau `"group"`
    pub kind: String,
    pub name: String,
    /// untuk dm: `[peer_id]`; untuk group: semua anggota termasuk diri sendiri
    pub members: Vec<String>,
    pub last_ts: u64,
    pub unread: i64,
    pub manual_unread: bool,
    pub preview: String,
}

pub struct Store {
    db: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let db = Connection::open(path)
            .with_context(|| format!("buka db {}", path.display()))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        db.execute_batch(SCHEMA)?;
        // Migrasi ringan: DB lama belum punya kolom `read`. ALTER TABLE gagal kalau
        // kolom sudah ada, jadi error-nya sengaja diabaikan.
        let _ = db.execute("ALTER TABLE messages ADD COLUMN read INTEGER NOT NULL DEFAULT 0", []);
        Ok(Self { db: Mutex::new(db) })
    }

    pub fn open_in_memory() -> Result<Self> {
        let db = Connection::open_in_memory()?;
        db.execute_batch(SCHEMA)?;
        Ok(Self { db: Mutex::new(db) })
    }

    pub fn save_message(&self, m: &StoredMessage) -> Result<()> {
        let db = self.db.lock().unwrap();
        if let Some(a) = &m.attachment {
            db.execute(
                "INSERT INTO attachments (msg_id,file_name,file_size,file_path,sha256,mime)
                 VALUES (?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(msg_id) DO UPDATE SET
                   file_name=excluded.file_name, file_size=excluded.file_size,
                   file_path=excluded.file_path, sha256=excluded.sha256, mime=excluded.mime",
                params![m.id, a.name, a.size as i64, a.path, a.sha256, a.mime],
            )?;
        }
        db.execute(
            "INSERT OR REPLACE INTO messages (id,thread_id,from_id,from_name,body,ts,mine,delivered,read)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                m.id,
                m.thread_id,
                m.from_id,
                m.from_name,
                m.body,
                m.ts as i64,
                m.mine as i64,
                m.delivered as i64,
                m.read as i64
            ],
        )?;
        // thread ikut ter-update (preview + waktu terakhir)
        let preview = match &m.attachment {
            Some(a) => format!("📎 {}", a.name),
            None => preview_of(&m.body),
        };
        db.execute(
            "UPDATE threads SET last_ts=?2, preview=?3 WHERE id=?1",
            params![m.thread_id, m.ts as i64, preview],
        )?;
        Ok(())
    }

    /// Upsert metadata thread tanpa menimpa waktu/preview kalau sudah ada.
    pub fn ensure_thread(&self, t: &Thread) -> Result<()> {
        let db = self.db.lock().unwrap();
        let members = serde_json::to_string(&t.members)?;
        db.execute(
            "INSERT INTO threads (id,kind,name,members,last_ts,unread,manual_unread,preview)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(id) DO UPDATE SET
               name=excluded.name,
               kind=excluded.kind,
               members=excluded.members,
               unread=MAX(threads.unread, excluded.unread)",
            params![
                t.id,
                t.kind,
                t.name,
                members,
                t.last_ts as i64,
                t.unread,
                t.manual_unread as i64,
                t.preview
            ],
        )?;
        Ok(())
    }

    pub fn threads(&self) -> Result<Vec<Thread>> {
        let db = self.db.lock().unwrap();
        let mut st = db.prepare(
            "SELECT id,kind,name,members,last_ts,unread,manual_unread,preview
             FROM threads ORDER BY last_ts DESC",
        )?;
        let rows = st.query_map([], |r| {
            let members: String = r.get(3)?;
            Ok(Thread {
                id: r.get(0)?,
                kind: r.get(1)?,
                name: r.get(2)?,
                members: serde_json::from_str(&members).unwrap_or_default(),
                last_ts: r.get::<_, i64>(4)? as u64,
                unread: r.get(5)?,
                manual_unread: r.get::<_, i64>(6)? != 0,
                preview: r.get(7)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn thread(&self, id: &str) -> Result<Option<Thread>> {
        Ok(self.threads()?.into_iter().find(|t| t.id == id))
    }

    pub fn messages(&self, thread_id: &str, limit: usize) -> Result<Vec<StoredMessage>> {
        let db = self.db.lock().unwrap();
        let mut st = db.prepare(
            "SELECT m.id,m.thread_id,m.from_id,m.from_name,m.body,m.ts,m.mine,m.delivered,m.read,
                    a.file_name,a.file_size,a.file_path,a.sha256,a.mime
             FROM messages m LEFT JOIN attachments a ON a.msg_id = m.id
             WHERE m.thread_id=?1 ORDER BY m.ts DESC LIMIT ?2",
        )?;
        let rows = st.query_map(params![thread_id, limit as i64], |r| {
            let file_name: Option<String> = r.get(9)?;
            let attachment = match file_name {
                Some(name) => Some(Attachment {
                    name,
                    size: r.get::<_, i64>(10)? as u64,
                    path: r.get(11)?,
                    sha256: r.get(12)?,
                    mime: r.get(13)?,
                }),
                None => None,
            };
            Ok(StoredMessage {
                id: r.get(0)?,
                thread_id: r.get(1)?,
                from_id: r.get(2)?,
                from_name: r.get(3)?,
                body: r.get(4)?,
                ts: r.get::<_, i64>(5)? as u64,
                mine: r.get::<_, i64>(6)? != 0,
                delivered: r.get::<_, i64>(7)? != 0,
                read: r.get::<_, i64>(8)? != 0,
                attachment,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        out.reverse(); // kronologis
        Ok(out)
    }

    /// Tambah counter unread (pesan masuk saat thread tidak dibuka).
    pub fn bump_unread(&self, thread_id: &str) -> Result<()> {
        let db = self.db.lock().unwrap();
        db.execute(
            "UPDATE threads SET unread = unread + 1 WHERE id=?1",
            params![thread_id],
        )?;
        Ok(())
    }

    pub fn set_unread(&self, thread_id: &str, count: i64, manual: bool) -> Result<()> {
        let db = self.db.lock().unwrap();
        db.execute(
            "UPDATE threads SET unread=?2, manual_unread=?3 WHERE id=?1",
            params![thread_id, count, manual as i64],
        )?;
        Ok(())
    }

    pub fn mark_read(&self, thread_id: &str) -> Result<()> {
        self.set_unread(thread_id, 0, false)
    }

    pub fn has_unread(&self, thread_id: &str) -> Result<bool> {
        let db = self.db.lock().unwrap();
        let n: Option<i64> = db
            .query_row(
                "SELECT (unread>0 OR manual_unread>0) FROM threads WHERE id=?1",
                params![thread_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(n.unwrap_or(0) != 0)
    }

    /// Tandai pesan sendiri (sampai ts tertentu) sudah delivered.
    pub fn mark_delivered(&self, thread_id: &str, up_to_ts: u64) -> Result<()> {
        let db = self.db.lock().unwrap();
        db.execute(
            "UPDATE messages SET delivered=1 WHERE thread_id=?1 AND mine=1 AND ts<=?2",
            params![thread_id, up_to_ts as i64],
        )?;
        Ok(())
    }

    /// Tandai pesan sendiri sudah **dibaca** peer (read receipt).
    /// Read juga menyiratkan delivered.
    pub fn mark_receipt_read(&self, thread_id: &str, up_to_ts: u64) -> Result<()> {
        let db = self.db.lock().unwrap();
        db.execute(
            "UPDATE messages SET delivered=1, read=1 WHERE thread_id=?1 AND mine=1 AND ts<=?2",
            params![thread_id, up_to_ts as i64],
        )?;
        Ok(())
    }

    pub fn clear_thread(&self, thread_id: &str) -> Result<()> {
        self.clear_thread_ex(thread_id, false).map(|_| ())
    }

    /// Hapus riwayat satu thread.
    ///
    /// `delete_files=true` juga menghapus file lampiran dari disk. Default
    /// `false`: file yang sudah diterima adalah milik user (ada di Downloads),
    /// jadi menghapus riwayat chat tidak otomatis membuang dokumennya.
    ///
    /// Mengembalikan jumlah file yang dihapus.
    pub fn clear_thread_ex(&self, thread_id: &str, delete_files: bool) -> Result<usize> {
        let paths: Vec<String> = if delete_files {
            let db = self.db.lock().unwrap();
            let mut stmt = db.prepare(
                "SELECT a.file_path FROM attachments a
                 JOIN messages m ON m.id = a.msg_id
                 WHERE m.thread_id = ?1 AND a.file_path IS NOT NULL AND a.file_path != ''",
            )?;
            let rows = stmt.query_map(params![thread_id], |r| r.get::<_, String>(0))?;
            rows.filter_map(|r| r.ok()).collect()
        } else {
            Vec::new()
        };

        {
            let db = self.db.lock().unwrap();
            db.execute(
                "DELETE FROM attachments WHERE msg_id IN (SELECT id FROM messages WHERE thread_id=?1)",
                params![thread_id],
            )?;
            db.execute("DELETE FROM messages WHERE thread_id=?1", params![thread_id])?;
            db.execute(
                "UPDATE threads SET preview='', unread=0, manual_unread=0 WHERE id=?1",
                params![thread_id],
            )?;
        }

        // Hapus file di luar lock DB supaya I/O tidak menahan thread lain.
        let mut removed = 0usize;
        for p in paths {
            // Hanya hapus file biasa; folder atau path aneh dilewati.
            if let Ok(meta) = std::fs::metadata(&p) {
                if meta.is_file() && std::fs::remove_file(&p).is_ok() {
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }

    pub fn delete_thread(&self, thread_id: &str) -> Result<()> {
        let db = self.db.lock().unwrap();
        db.execute(
            "DELETE FROM attachments WHERE msg_id IN (SELECT id FROM messages WHERE thread_id=?1)",
            params![thread_id],
        )?;
        db.execute("DELETE FROM messages WHERE thread_id=?1", params![thread_id])?;
        db.execute("DELETE FROM threads WHERE id=?1", params![thread_id])?;
        Ok(())
    }

    pub fn set_setting(&self, k: &str, v: &str) -> Result<()> {
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT INTO settings (k,v) VALUES (?1,?2)
             ON CONFLICT(k) DO UPDATE SET v=excluded.v",
            params![k, v],
        )?;
        Ok(())
    }

    pub fn get_setting(&self, k: &str) -> Result<Option<String>> {
        let db = self.db.lock().unwrap();
        Ok(db
            .query_row("SELECT v FROM settings WHERE k=?1", params![k], |r| r.get(0))
            .optional()?)
    }
}

fn preview_of(body: &str) -> String {
    let line = body.lines().next().unwrap_or("");
    let mut s: String = line.chars().take(120).collect();
    if line.chars().count() > 120 {
        s.push('…');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, thread: &str, ts: u64, mine: bool, body: &str) -> StoredMessage {
        StoredMessage {
            id: id.into(),
            thread_id: thread.into(),
            from_id: if mine { "me".into() } else { "peer".into() },
            from_name: "x".into(),
            body: body.into(),
            ts,
            mine,
            delivered: false,
            read: false,
            attachment: None,
        }
    }

    #[test]
    fn messages_roundtrip_in_order() {
        let s = Store::open_in_memory().unwrap();
        s.ensure_thread(&Thread {
            id: "p1".into(),
            kind: "dm".into(),
            name: "peer".into(),
            members: vec!["p1".into()],
            last_ts: 0,
            unread: 0,
            manual_unread: false,
            preview: String::new(),
        })
        .unwrap();
        s.save_message(&msg("m1", "p1", 10, true, "halo")).unwrap();
        s.save_message(&msg("m2", "p1", 20, false, "hai juga")).unwrap();
        let got = s.messages("p1", 50).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].body, "hai juga");
        let t = s.thread("p1").unwrap().unwrap();
        assert_eq!(t.preview, "hai juga");
        assert_eq!(t.last_ts, 20);
    }

    #[test]
    fn unread_and_clear() {
        let s = Store::open_in_memory().unwrap();
        s.ensure_thread(&Thread {
            id: "g".into(),
            kind: "group".into(),
            name: "tim".into(),
            members: vec![],
            last_ts: 0,
            unread: 0,
            manual_unread: false,
            preview: String::new(),
        })
        .unwrap();
        s.bump_unread("g").unwrap();
        s.bump_unread("g").unwrap();
        assert!(s.has_unread("g").unwrap());
        s.set_unread("g", 0, true).unwrap();
        assert!(s.has_unread("g").unwrap());
        s.mark_read("g").unwrap();
        assert!(!s.has_unread("g").unwrap());
        s.clear_thread("g").unwrap();
        assert!(s.messages("g", 10).unwrap().is_empty());
    }

    #[test]
    fn attachment_tersimpan_dan_ikut_terhapus() {
        let s = Store::open_in_memory().unwrap();
        s.ensure_thread(&Thread {
            id: "p2".into(),
            kind: "dm".into(),
            name: "peer".into(),
            members: vec!["p2".into()],
            last_ts: 0,
            unread: 0,
            manual_unread: false,
            preview: String::new(),
        })
        .unwrap();
        let mut m = msg("m9", "p2", 30, true, "");
        m.attachment = Some(Attachment {
            name: "laporan.pdf".into(),
            size: 1234,
            path: "/tmp/laporan.pdf".into(),
            sha256: "deadbeef".into(),
            mime: String::new(),
        });
        s.save_message(&m).unwrap();

        let got = s.messages("p2", 10).unwrap();
        assert_eq!(got.len(), 1);
        let a = got[0].attachment.as_ref().expect("lampiran hilang");
        assert_eq!(a.name, "laporan.pdf");
        assert_eq!(a.size, 1234);
        assert_eq!(a.sha256, "deadbeef");
        // preview thread menampilkan nama file, bukan body kosong
        assert_eq!(s.thread("p2").unwrap().unwrap().preview, "📎 laporan.pdf");

        s.clear_thread("p2").unwrap();
        assert!(s.messages("p2", 10).unwrap().is_empty());
        // baris lampiran ikut terhapus, tidak menggantung
        let n: i64 = {
            let db = s.db.lock().unwrap();
            db.query_row("SELECT COUNT(*) FROM attachments", [], |r| r.get(0)).unwrap()
        };
        assert_eq!(n, 0);
    }

    #[test]
    fn read_receipt_terpisah_dari_delivered() {
        let s = Store::open_in_memory().unwrap();
        s.ensure_thread(&Thread {
            id: "p3".into(), kind: "dm".into(), name: "peer".into(), members: vec!["p3".into()],
            last_ts: 0, unread: 0, manual_unread: false, preview: String::new(),
        }).unwrap();
        s.save_message(&msg("r1", "p3", 100, true, "satu")).unwrap();
        s.save_message(&msg("r2", "p3", 200, true, "dua")).unwrap();

        // belum ada apa-apa
        let m = s.messages("p3", 10).unwrap();
        assert!(!m[0].delivered && !m[0].read);

        // sampai perangkat peer (ack) -> delivered, belum read
        s.mark_delivered("p3", 150).unwrap();
        let m = s.messages("p3", 10).unwrap();
        assert!(m[0].delivered && !m[0].read, "pesan 1 harus delivered tapi belum read");
        assert!(!m[1].delivered, "pesan 2 belum tersentuh");

        // dibaca peer -> read (dan delivered)
        s.mark_receipt_read("p3", 250).unwrap();
        let m = s.messages("p3", 10).unwrap();
        assert!(m[0].read && m[0].delivered);
        assert!(m[1].read && m[1].delivered, "pesan 2 juga sudah dibaca");
    }

    #[test]
    fn clear_thread_dengan_file_menghapus_lampiran() {
        let s = Store::open_in_memory().unwrap();
        s.ensure_thread(&Thread {
            id: "pc".into(), kind: "dm".into(), name: "peer".into(), members: vec!["pc".into()],
            last_ts: 0, unread: 0, manual_unread: false, preview: String::new(),
        }).unwrap();

        // dua file nyata di disk
        let dir = std::env::temp_dir().join(format!("lanchat-clear-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f1 = dir.join("a.txt");
        let f2 = dir.join("b.txt");
        std::fs::write(&f1, b"satu").unwrap();
        std::fs::write(&f2, b"dua").unwrap();

        for (id, path) in [("m-a", &f1), ("m-b", &f2)] {
            let mut m = msg(id, "pc", 100, true, "");
            m.attachment = Some(Attachment {
                name: path.file_name().unwrap().to_string_lossy().to_string(),
                size: 4,
                path: path.to_string_lossy().to_string(),
                sha256: "x".into(),
                mime: String::new(),
            });
            s.save_message(&m).unwrap();
        }

        // tanpa delete_files: file tetap ada
        let n = s.clear_thread_ex("pc", false).unwrap();
        assert_eq!(n, 0, "tanpa flag, tidak ada file dihapus");
        assert!(f1.exists() && f2.exists(), "file harus tetap ada");

        // ulangi lalu hapus dengan file
        for (id, path) in [("m-c", &f1), ("m-d", &f2)] {
            let mut m = msg(id, "pc", 200, true, "");
            m.attachment = Some(Attachment {
                name: path.file_name().unwrap().to_string_lossy().to_string(),
                size: 4,
                path: path.to_string_lossy().to_string(),
                sha256: "x".into(),
                mime: String::new(),
            });
            s.save_message(&m).unwrap();
        }
        let n = s.clear_thread_ex("pc", true).unwrap();
        assert_eq!(n, 2, "dua file harus dihapus");
        assert!(!f1.exists() && !f2.exists(), "file harus terhapus dari disk");
        assert!(s.messages("pc", 10).unwrap().is_empty(), "riwayat harus kosong");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_persist() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.get_setting("theme").unwrap().is_none());
        s.set_setting("theme", "dark").unwrap();
        s.set_setting("theme", "light").unwrap();
        assert_eq!(s.get_setting("theme").unwrap().unwrap(), "light");
    }
}