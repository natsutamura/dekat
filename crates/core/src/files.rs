//! Transfer file: offer → accept → chunk → done.
//!
//! Alur (sisi pengirim):
//! 1. baca file, hitung sha256 + jumlah chunk
//! 2. kirim [`Frame::FileOffer`]
//! 3. tunggu [`Frame::FileAccept`]; kalau ditolak, berhenti (tidak buang bandwidth)
//! 4. kirim [`Frame::FileChunk`] berurutan, dengan batas laju supaya koneksi chat
//!    tidak tersendat
//! 5. tunggu [`Frame::FileDone`] dari penerima
//!
//! Sisi penerima menulis ke `<data_dir>/files/partial/<sha256>.part` sambil
//! memverifikasi sha256 di akhir; kalau cocok, file dipindah ke
//! `<data_dir>/files/<msg_id>-<nama-aman>`.
//!
//! **Resume**: partial diberi nama menurut sha256 isi file, bukan menurut msg_id.
//! Kalau koneksi putus di tengah, pengiriman ulang file yang sama akan menemukan
//! partial itu dan melanjutkan dari byte terakhir — tidak mulai dari nol. Penerima
//! melaporkan posisinya lewat `FileAccept.have`, jadi pengirim yang menentukan
//! mulai dari potongan mana.
//!
//! Batasan M3: satu file per pesan, penerima harus online.

use crate::proto::{
    now_ms, safe_filename, Frame, FILE_CHUNK_BYTES, MAX_FILE_BYTES, MAX_FRAME,
};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// Metadata file yang siap dikirim.
#[derive(Debug, Clone)]
pub struct FileMeta {
    pub name: String,
    pub size: u64,
    pub sha256: String,
    pub chunks: u32,
}

/// Status transfer yang sedang berjalan, dipakai untuk progres di UI.
#[derive(Debug, Clone, Default)]
pub struct Transfer {
    pub name: String,
    pub size: u64,
    pub sent_chunks: u32,
    pub total_chunks: u32,
    pub received_bytes: u64,
}

/// Asal & tujuan sebuah transfer — dibutuhkan saat file selesai supaya
/// konfirmasi `FileDone` dikirim ke peer yang benar (bukan koneksi sembarang).
#[derive(Debug, Clone, Default)]
pub struct FileContext {
    pub from: String,
    pub from_name: String,
    pub to: String,
    pub members: Option<Vec<String>>,
    pub ts: u64,
}

/// Transfer masuk yang sedang menunggu potongan.
struct Inbound {
    /// File tujuan akhir (belum ada sampai transfer selesai).
    path: PathBuf,
    /// File setengah jadi yang dipakai untuk resume.
    partial: PathBuf,
    file: File,
    digest: Sha256,
    expected_sha: String,
    expected_size: u64,
    received: u64,
    next_idx: u32,
    ctx: FileContext,
}

/// Pengelola transfer file per node. Thread-safe karena dipakai banyak task koneksi.
#[derive(Clone, Default)]
pub struct FileTransfer {
    /// transfer keluar yang sedang jalan, key = msg_id
    pub outgoing: Arc<Mutex<HashMap<String, Transfer>>>,
    /// transfer masuk, key = msg_id
    pub incoming: Arc<Mutex<HashMap<String, Transfer>>>,
    inbound: Arc<Mutex<HashMap<String, Inbound>>>,
}

impl FileTransfer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Siapkan metadata + validasi file sebelum ditawarkan ke peer.
    pub fn prepare(path: &Path) -> Result<FileMeta> {
        let meta = std::fs::metadata(path).with_context(|| format!("baca {}", path.display()))?;
        if !meta.is_file() {
            bail!("bukan file biasa: {}", path.display());
        }
        if meta.len() > MAX_FILE_BYTES {
            bail!(
                "file terlalu besar ({} > batas {})",
                meta.len(),
                MAX_FILE_BYTES
            );
        }
        if meta.len() == 0 {
            bail!("file kosong tidak bisa dikirim");
        }
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut f = File::open(path)?;
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let size = meta.len();
        let chunks = size.div_ceil(FILE_CHUNK_BYTES as u64) as u32;
        Ok(FileMeta {
            name: path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "file".into()),
            size,
            sha256: hex::encode(hasher.finalize()),
            chunks,
        })
    }

    /// Kirim seluruh isi file dalam potongan base64 lewat closure pengirim.
    ///
    /// `send` dipanggil untuk setiap frame; implementasinya di node adalah
    /// `try_send` ke kanal writer koneksi peer.
    pub fn stream_file<F>(&self, msg_id: &str, path: &Path, meta: &FileMeta, mut send: F) -> Result<()>
    where
        F: FnMut(Frame) -> bool,
    {
        self.stream_file_from(msg_id, path, meta, 0, send)
    }

    /// Sama seperti [`Self::stream_file`], tapi mulai dari byte ke-`start_offset`.
    ///
    /// Dipakai untuk resume: penerima melaporkan `have`, pengirim langsung
    /// melompat ke potongan itu, jadi byte yang sudah terkirim tidak diulang.
    pub fn stream_file_from<F>(
        &self,
        msg_id: &str,
        path: &Path,
        meta: &FileMeta,
        start_offset: u64,
        mut send: F,
    ) -> Result<()>
    where
        F: FnMut(Frame) -> bool,
    {
        {
            let mut out = self.outgoing.lock().unwrap();
            out.insert(
                msg_id.to_string(),
                Transfer {
                    name: meta.name.clone(),
                    size: meta.size,
                    sent_chunks: 0,
                    total_chunks: meta.chunks,
                    received_bytes: 0,
                },
            );
        }
        let result = (|| -> Result<()> {
            // Offset harus kelipatan potongan; kalau tidak, mulai dari awal saja
            // (lebih baik kirim ulang daripada menghasilkan file rusak).
            let mut start = start_offset;
            if start % FILE_CHUNK_BYTES as u64 != 0 || start >= meta.size {
                start = 0;
            }
            let mut f = File::open(path)?;
            if start > 0 {
                use std::io::Seek;
                f.seek(std::io::SeekFrom::Start(start))?;
            }
            let mut buf = vec![0u8; FILE_CHUNK_BYTES];
            let mut idx = (start / FILE_CHUNK_BYTES as u64) as u32;
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                let data = B64.encode(&buf[..n]);
                // frame gabungan tidak boleh melewati MAX_FRAME
                if data.len() + 256 > MAX_FRAME {
                    bail!("potongan terlalu besar setelah base64");
                }
                let frame = Frame::FileChunk {
                    id: msg_id.to_string(),
                    idx,
                    data,
                };
                if !send(frame) {
                    bail!("koneksi peer tertutup saat mengirim file");
                }
                idx += 1;
                if let Some(t) = self.outgoing.lock().unwrap().get_mut(msg_id) {
                    t.sent_chunks = idx;
                    t.received_bytes = idx as u64 * FILE_CHUNK_BYTES as u64;
                }
            }
            if idx != meta.chunks {
                bail!("jumlah potongan tidak sesuai ({} != {})", idx, meta.chunks);
            }
            Ok(())
        })();
        if result.is_err() {
            self.outgoing.lock().unwrap().remove(msg_id);
        }
        result
    }

    /// Mulai menerima file. Dipanggil saat `FileOffer` datang dan diterima.
    pub fn begin_inbound(&self, dir: &Path, msg_id: &str, name: &str, size: u64, sha256: &str, total_chunks: u32) -> Result<(PathBuf, u64)> {
        self.begin_inbound_ctx(dir, msg_id, name, size, sha256, total_chunks, FileContext::default())
    }

    /// Sama seperti [`Self::begin_inbound`], tapi menyimpan konteks pengirim.
    #[allow(clippy::too_many_arguments)]
    pub fn begin_inbound_ctx(
        &self,
        dir: &Path,
        msg_id: &str,
        name: &str,
        size: u64,
        sha256: &str,
        total_chunks: u32,
        ctx: FileContext,
    ) -> Result<(PathBuf, u64)> {
        if size > MAX_FILE_BYTES {
            bail!("file ditolak: {} byte melebihi batas", size);
        }
        std::fs::create_dir_all(dir)?;
        let safe = safe_filename(name);
        // nama file unik per pesan, jadi pengiriman ulang tidak menimpa file lain
        let path = dir.join(format!("{msg_id}-{safe}"));

        // Partial dinamai menurut sha256 isi file: pengiriman ulang file yang sama
        // (walau msg_id baru) tetap menemukan sisa transfer sebelumnya.
        let partial_dir = dir.join("partial");
        std::fs::create_dir_all(&partial_dir)?;
        let partial = partial_dir.join(format!("{}.part", sha256));

        // Lanjutkan hanya kalau sisa partial masih valid: ukurannya kelipatan
        // potongan dan tidak melebihi ukuran file.
        let mut received = 0u64;
        if let Ok(m) = std::fs::metadata(&partial) {
            let have = m.len();
            if have > 0 && have < size && have % FILE_CHUNK_BYTES as u64 == 0 {
                received = have;
            }
        }
        let mut digest = Sha256::new();
        let file = if received > 0 {
            // Hash ulang byte yang sudah ada supaya verifikasi akhir tetap sah.
            let mut f = File::open(&partial)?;
            let mut buf = vec![0u8; 64 * 1024];
            let mut left = received;
            while left > 0 {
                let want = (left as usize).min(buf.len());
                let n = f.read(&mut buf[..want])?;
                if n == 0 { break }
                digest.update(&buf[..n]);
                left -= n as u64;
            }
            drop(f);
            let f = std::fs::OpenOptions::new().append(true).open(&partial)?;
            f
        } else {
            let f = File::create(&partial).with_context(|| format!("buat {}", partial.display()))?;
            f
        };

        let inbound = Inbound {
            path: path.clone(),
            partial: partial.clone(),
            file,
            digest,
            expected_sha: sha256.to_string(),
            expected_size: size,
            received,
            next_idx: (received / FILE_CHUNK_BYTES as u64) as u32,
            ctx,
        };
        self.inbound.lock().unwrap().insert(msg_id.to_string(), inbound);
        self.incoming.lock().unwrap().insert(
            msg_id.to_string(),
            Transfer {
                name: safe,
                size,
                sent_chunks: 0,
                total_chunks,
                received_bytes: received,
            },
        );
        Ok((path, received))
    }

    /// Tulis satu potongan. Mengembalikan `Some((path, konteks))` kalau file selesai & utuh.
    pub fn write_chunk(
        &self,
        msg_id: &str,
        idx: u32,
        data_b64: &str,
    ) -> Result<Option<(PathBuf, FileContext)>> {
        let bytes = B64
            .decode(data_b64.as_bytes())
            .context("potongan bukan base64 valid")?;

        let mut map = self.inbound.lock().unwrap();
        let inb = map
            .get_mut(msg_id)
            .ok_or_else(|| anyhow!("tidak ada transfer masuk untuk {msg_id}"))?;

        // potongan harus berurutan; frame nyasar/duplikat ditolak
        if idx != inb.next_idx {
            bail!(
                "potongan tidak berurutan (dapat {idx}, harus {})",
                inb.next_idx
            );
        }
        if inb.received + bytes.len() as u64 > inb.expected_size {
            bail!("file melebihi ukuran yang dijanjikan");
        }
        inb.file.write_all(&bytes)?;
        inb.digest.update(&bytes);
        inb.received += bytes.len() as u64;
        inb.next_idx += 1;

        if let Some(t) = self.incoming.lock().unwrap().get_mut(msg_id) {
            t.received_bytes = inb.received;
            t.sent_chunks = inb.next_idx;
        }

        if inb.received < inb.expected_size {
            return Ok(None);
        }

        // selesai: flush, verifikasi sha256
        inb.file.flush()?;
        let actual = hex::encode(inb.digest.clone().finalize());
        let inb = map.remove(msg_id).unwrap();
        self.incoming.lock().unwrap().remove(msg_id);

        if actual != inb.expected_sha {
            // Partial sengaja TIDAK dihapus di sini kalau isinya sudah lengkap tapi
            // hash salah — itu tanda korupsi nyata, jadi lebih baik dibuang.
            let _ = std::fs::remove_file(&inb.partial);
            let _ = std::fs::remove_file(&inb.path);
            bail!(
                "sha256 tidak cocok (dapat {actual}, harus {}) — file dihapus",
                inb.expected_sha
            );
        }
        // Pindahkan partial ke nama akhir hanya setelah terbukti utuh.
        drop(inb.file);
        if let Err(e) = std::fs::rename(&inb.partial, &inb.path) {
            // Fallback kalau beda filesystem.
            std::fs::copy(&inb.partial, &inb.path).with_context(|| format!("salin ke {}", inb.path.display()))?;
            let _ = std::fs::remove_file(&inb.partial);
            tracing::debug!("rename partial gagal ({e}), pakai copy");
        }
        Ok(Some((inb.path, inb.ctx)))
    }

    /// Siapa pengirim transfer masuk ini (buat mengirim FileDone gagal).
    pub fn incoming_sender(&self, msg_id: &str) -> Option<String> {
        self.inbound
            .lock()
            .unwrap()
            .get(msg_id)
            .map(|i| i.ctx.from.clone())
    }

    /// Batalkan transfer masuk & hapus file setengah jadi.
    pub fn abort_inbound(&self, msg_id: &str) {
        let removed = self.inbound.lock().unwrap().remove(msg_id);
        self.incoming.lock().unwrap().remove(msg_id);
        self.outgoing.lock().unwrap().remove(msg_id);
        if let Some(inb) = removed {
            let _ = std::fs::remove_file(&inb.path);
            // Partial juga dihapus saat abort eksplisit (user membatalkan).
            let _ = std::fs::remove_file(&inb.partial);
        }
    }

    /// Hapus catatan transfer keluar (sukses/gagal).
    pub fn finish_outgoing(&self, msg_id: &str) {
        self.outgoing.lock().unwrap().remove(msg_id);
    }

    pub fn outgoing_progress(&self, msg_id: &str) -> Option<Transfer> {
        self.outgoing.lock().unwrap().get(msg_id).cloned()
    }

    pub fn incoming_progress(&self, msg_id: &str) -> Option<Transfer> {
        self.incoming.lock().unwrap().get(msg_id).cloned()
    }
}

/// Hitung sha256 file utuh (dipakai test).
/// sha256 dari potongan byte di memori (dipakai test & verifikasi kecil).
pub fn sha256_file_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut f = File::open(path)?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Timestamp helper dipakai UI untuk label "baru saja".
pub fn now() -> u64 {
    now_ms()
}

#[cfg(test)]
mod tests {
    /// Hitung file (rekursif), bukan direktori — layout sekarang punya `partial/`.
    fn count_files(dir: &Path) -> usize {
        let mut n = 0;
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                if e.path().is_dir() {
                    n += count_files(&e.path());
                } else {
                    n += 1;
                }
            }
        }
        n
    }

    use super::*;
    use std::io::Seek;

    fn tmpdir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "lanchat-ft-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn write_sample(dir: &Path, name: &str, size: usize) -> PathBuf {
        let p = dir.join(name);
        let mut f = File::create(&p).unwrap();
        let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        f.write_all(&data).unwrap();
        p
    }

    #[test]
    fn prepare_menghitung_chunk_dan_hash() {
        let dir = tmpdir("prepare");
        let path = write_sample(&dir, "sample.bin", 300 * 1024);
        let meta = FileTransfer::prepare(&path).unwrap();
        assert_eq!(meta.name, "sample.bin");
        assert_eq!(meta.size, 300 * 1024);
        assert_eq!(meta.chunks, 3); // 128KB * 3
        assert_eq!(meta.sha256, sha256_file(&path).unwrap());
    }

    #[test]
    fn prepare_menolak_file_kosong() {
        let dir = tmpdir("empty");
        let path = dir.join("kosong.txt");
        File::create(&path).unwrap();
        assert!(FileTransfer::prepare(&path).is_err());
    }

    #[test]
    fn transfer_utuh_antar_dua_sisi() {
        let src = tmpdir("src");
        let dst = tmpdir("dst");
        let path = write_sample(&src, "foto.bin", 300 * 1024 + 77);
        let meta = FileTransfer::prepare(&path).unwrap();
        let ft = FileTransfer::new();
        let msg_id = "m1";

        ft.begin_inbound(&dst, msg_id, &meta.name, meta.size, &meta.sha256, meta.chunks)
            .unwrap();

        // tangkap frame yang "dikirim" lalu tulis ke sisi penerima
        let got: Arc<Mutex<Vec<(u32, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let g = got.clone();
        ft.stream_file(msg_id, &path, &meta, move |f| {
            if let Frame::FileChunk { idx, data, .. } = f {
                g.lock().unwrap().push((idx, data));
            }
            true
        })
        .unwrap();

        let frames = got.lock().unwrap().clone();
        assert_eq!(frames.len() as u32, meta.chunks);

        let mut done_path = None;
        for (idx, data) in frames {
            if let Some((p, _ctx)) = ft.write_chunk(msg_id, idx, &data).unwrap() {
                done_path = Some(p);
            }
        }
        let done = done_path.expect("file tidak pernah selesai");
        assert_eq!(sha256_file(&done).unwrap(), meta.sha256);
        assert_eq!(std::fs::metadata(&done).unwrap().len(), meta.size);
        // nama file diberi prefix msg_id supaya unik
        assert!(done.file_name().unwrap().to_string_lossy().ends_with("foto.bin"));
    }

    #[test]
    fn chunk_rusak_ditolak_dan_file_dihapus() {
        let src = tmpdir("corrupt-src");
        let dst = tmpdir("corrupt-dst");
        let path = write_sample(&src, "data.bin", 200 * 1024);
        let meta = FileTransfer::prepare(&path).unwrap();
        let ft = FileTransfer::new();
        let msg_id = "m2";
        ft.begin_inbound(&dst, msg_id, &meta.name, meta.size, &meta.sha256, meta.chunks)
            .unwrap();

        let mut frames = Vec::new();
        ft.stream_file(msg_id, &path, &meta, |f| {
            if let Frame::FileChunk { idx, data, .. } = f {
                frames.push((idx, data));
            }
            true
        })
        .unwrap();

        // rusak potongan terakhir TANPA mengubah panjangnya, jadi ukuran total
        // tetap cocok dan yang menangkap harus verifikasi sha256
        let last = frames.len() - 1;
        let mut bytes = B64.decode(frames[last].1.as_bytes()).unwrap();
        bytes[0] ^= 0xFF;
        frames[last].1 = B64.encode(&bytes);

        let mut err = None;
        for (idx, data) in frames {
            if let Err(e) = ft.write_chunk(msg_id, idx, &data) {
                err = Some(e.to_string());
            }
        }
        let msg = err.unwrap_or_else(|| "harusnya error".into());
        assert!(msg.contains("tidak cocok"), "pesan error: {msg}");
        // tidak ada file tersisa di folder tujuan (folder `partial` boleh ada,
        // tapi harus kosong — partial yang sha-nya salah dibuang)
        let sisa = count_files(&dst);
        assert_eq!(sisa, 0, "file rusak harus dihapus");
    }

    #[test]
    fn potongan_tidak_berurutan_ditolak() {
        let src = tmpdir("order-src");
        let dst = tmpdir("order-dst");
        let path = write_sample(&src, "urut.bin", 200 * 1024);
        let meta = FileTransfer::prepare(&path).unwrap();
        let ft = FileTransfer::new();
        ft.begin_inbound(&dst, "m3", &meta.name, meta.size, &meta.sha256, meta.chunks)
            .unwrap();
        let mut frames = Vec::new();
        ft.stream_file("m3", &path, &meta, |f| {
            if let Frame::FileChunk { idx, data, .. } = f {
                frames.push((idx, data));
            }
            true
        })
        .unwrap();
        // kirim potongan kedua lebih dulu
        let err = ft.write_chunk("m3", frames[1].0, &frames[1].1).unwrap_err();
        assert!(err.to_string().contains("tidak berurutan"));
    }

    #[test]
    fn abort_menghapus_file_setengah_jadi() {
        let src = tmpdir("abort-src");
        let dst = tmpdir("abort-dst");
        let path = write_sample(&src, "setengah.bin", 300 * 1024);
        let meta = FileTransfer::prepare(&path).unwrap();
        let ft = FileTransfer::new();
        ft.begin_inbound(&dst, "m4", &meta.name, meta.size, &meta.sha256, meta.chunks)
            .unwrap();
        assert_eq!(count_files(&dst), 1, "harus ada 1 partial saat transfer jalan");
        ft.abort_inbound("m4");
        assert_eq!(count_files(&dst), 0, "abort harus menghapus partial");
        assert!(ft.incoming_progress("m4").is_none());
    }

    #[test]
    fn nama_file_dari_jaringan_tidak_bisa_keluar_folder() {
        let dst = tmpdir("traversal");
        let ft = FileTransfer::new();
        let (p, _have) = ft
            .begin_inbound(&dst, "m5", "../../../etc/passwd", 10, "x", 1)
            .unwrap();
        // harus tetap di dalam folder tujuan
        assert!(p.starts_with(&dst), "path keluar folder: {}", p.display());
        assert_eq!(p.file_name().unwrap().to_string_lossy(), "m5-passwd");
    }

    #[test]
    fn progress_terbaca() {
        let src = tmpdir("prog-src");
        let dst = tmpdir("prog-dst");
        let path = write_sample(&src, "p.bin", 300 * 1024);
        let meta = FileTransfer::prepare(&path).unwrap();
        let ft = FileTransfer::new();
        ft.begin_inbound(&dst, "m6", &meta.name, meta.size, &meta.sha256, meta.chunks)
            .unwrap();
        let mut frames = Vec::new();
        ft.stream_file("m6", &path, &meta, |f| {
            if let Frame::FileChunk { idx, data, .. } = f {
                frames.push((idx, data));
            }
            true
        })
        .unwrap();
        assert_eq!(ft.outgoing_progress("m6").unwrap().sent_chunks, meta.chunks);
        ft.write_chunk("m6", frames[0].0, &frames[0].1).unwrap();
        let inc = ft.incoming_progress("m6").unwrap();
        assert_eq!(inc.sent_chunks, 1);
        assert!(inc.received_bytes > 0);
    }

    #[test]
    fn sha_tidak_berubah_untuk_file_sama() {
        let dir = tmpdir("sha");
        let p = write_sample(&dir, "x.bin", 1000);
        let a = sha256_file(&p).unwrap();
        // sentuh file tanpa mengubah isi
        let mut f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
        f.seek(std::io::SeekFrom::Current(0)).unwrap();
        drop(f);
        assert_eq!(a, sha256_file(&p).unwrap());
    
    #[test]
    fn resume_lanjut_dari_partial() {
        let dir = std::env::temp_dir().join(format!("lanchat-resume-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // File uji 3 potongan penuh.
        let chunk = crate::proto::FILE_CHUNK_BYTES;
        let mut isi = vec![0u8; chunk * 3];
        for (i, b) in isi.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let sha = sha256_file_bytes(&isi);
        let total = isi.len() as u64;
        let chunks = (total / chunk as u64) as u32;

        // Simulasikan: potongan pertama sudah diterima sebelumnya (partial 1 chunk).
        let pdir = dir.join("partial");
        std::fs::create_dir_all(&pdir).unwrap();
        std::fs::write(pdir.join(format!("{sha}.part")), &isi[..chunk]).unwrap();

        let ft = FileTransfer::new();
        let (_path, have) = ft
            .begin_inbound(&dir, "m1", "f.bin", total, &sha, chunks)
            .expect("begin_inbound harus menerima resume");
        assert_eq!(have, chunk as u64, "harus melanjutkan dari 1 potongan");

        // Kirim potongan 2 dan 3 (indeks lanjut dari 1).
        let mut done = None;
        for idx in 1..3u32 {
            let from = idx as usize * chunk;
            let to = from + chunk;
            let b64 = B64.encode(&isi[from..to]);
            done = ft.write_chunk("m1", idx, &b64).unwrap();
        }
        let (path, _ctx) = done.expect("transfer harus selesai di potongan terakhir");
        assert_eq!(std::fs::read(&path).unwrap(), isi, "isi file harus utuh");
        assert_eq!(sha256_file_bytes(&std::fs::read(&path).unwrap()), sha);
        assert!(!pdir.join(format!("{sha}.part")).exists(), "partial harus hilang setelah selesai");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_tidak_valid_diabaikan() {
        let dir = std::env::temp_dir().join(format!("lanchat-resume-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let chunk = crate::proto::FILE_CHUNK_BYTES;
        let isi = vec![7u8; chunk * 2];
        let sha = sha256_file_bytes(&isi);
        let pdir = dir.join("partial");
        std::fs::create_dir_all(&pdir).unwrap();
        // Partial yang ukurannya BUKAN kelipatan potongan -> tidak boleh dipercaya.
        std::fs::write(pdir.join(format!("{sha}.part")), vec![9u8; 123]).unwrap();

        let ft = FileTransfer::new();
        let (_p, have) = ft
            .begin_inbound(&dir, "m2", "f.bin", isi.len() as u64, &sha, 2)
            .unwrap();
        assert_eq!(have, 0, "partial rusak harus diabaikan, mulai dari nol");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
}
