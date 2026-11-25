//! Node: gabungan discovery + transport + peer table + event stream.
//!
//! Aturan koneksi: untuk tiap pasangan node, hanya node dengan `id`
//! lebih kecil (lexicographic) yang melakukan dial. Hasilnya tepat satu
//! koneksi TCP bidirectional per pasangan — tidak ada pesan dobel.

use crate::{
    discovery::{self, Seen},
    files::FileTransfer,
    identity::Identity,
    proto::*,
    tls::Tls,
};
use anyhow::{bail, Context, Result};
use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{broadcast, mpsc},
};

#[derive(Clone, Debug)]
pub struct Config {
    pub name: String,
    /// 0 = pilih port bebas
    pub port: u16,
    pub data_dir: PathBuf,
    /// alamat `ip:port` yang di-announce secara unicast (fallback)
    pub seeds: Vec<SocketAddr>,
    /// matikan presence multicast (dipakai test supaya tidak saling ganggu)
    pub discovery: bool,
    /// Terima peer versi lama yang belum mendukung TLS.
    ///
    /// Default `false`: koneksi tanpa TLS **ditolak**. Dinyalakan hanya kalau user
    /// memang perlu bicara dengan build lama — dan itu membuka risiko penyadapan.
    pub allow_plaintext: bool,
}

impl Config {
    pub fn new(name: impl Into<String>, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            port: DEFAULT_APP_PORT,
            data_dir: data_dir.into(),
            seeds: Vec::new(),
            discovery: true,
            allow_plaintext: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub fp: String,
    pub os: String,
    pub addr: SocketAddr,
    pub last_seen: u64,
    pub status: Status,
    pub online: bool,
    /// Identitas peer sudah dibuktikan lewat tanda tangan di atas sesi TLS.
    /// `false` = peer lama (plaintext) yang diterima karena `allow_plaintext`.
    pub verified: bool,
    /// Avatar PNG base64 dari frame `Profile`, kalau peer mengirimnya.
    pub avatar: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Event {
    PeerUp(Peer),
    PeerDown(String),
    Text {
        id: String,
        from: String,
        from_name: String,
        to: String,
        body: String,
        ts: u64,
        mine: bool,
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
    PeerStatus {
        id: String,
        status: Status,
    },
    /// Profil peer berubah (nama dan/atau avatar).
    PeerProfile {
        id: String,
        name: String,
        avatar: Option<String>,
    },
    /// File ditawarkan (masuk). `accepted=false` berarti kita menolak.
    FileOffer {
        id: String,
        from: String,
        from_name: String,
        to: String,
        name: String,
        size: u64,
        ts: u64,
        mine: bool,
        members: Option<Vec<String>>,
    },
    /// Progres transfer (keluar atau masuk), dipakai UI buat progress bar.
    FileProgress {
        id: String,
        name: String,
        sent_bytes: u64,
        total_bytes: u64,
        outgoing: bool,
    },
    /// File selesai diterima & sha256 cocok, atau gagal.
    FileReceived {
        id: String,
        from: String,
        from_name: String,
        to: String,
        name: String,
        size: u64,
        path: String,
        ts: u64,
        members: Option<Vec<String>>,
    },
    FileFailed {
        id: String,
        name: String,
        reason: String,
    },
    /// Ada panggilan masuk (belum dijawab).
    CallIncoming {
        call_id: String,
        from: String,
        from_name: String,
        sdp: String,
        /// Peserta panggilan (termasuk pemanggil + diri sendiri). >2 = grup.
        members: Vec<String>,
    },
    /// Peer menerima ajakan kita, bawa SDP answer.
    CallAccepted {
        call_id: String,
        from: String,
        sdp: String,
    },
    CallRejected {
        call_id: String,
        from: String,
        reason: String,
    },
    /// Kandidat ICE dari peer.
    CallIce {
        call_id: String,
        from: String,
        candidate: String,
        sdp_mid: String,
        sdp_mline_index: u32,
    },
    CallEnded {
        call_id: String,
        from: String,
        reason: String,
    },
    Warn(String),
}

/// Profil yang bisa diubah user saat aplikasi berjalan.
struct MyProfile {
    name: String,
    fp: String,
    /// Avatar PNG base64 (tanpa prefix data URL), sudah diperkecil di klien.
    avatar: Option<String>,
}

struct Inner {
    id: String,
    data_dir: PathBuf,
    profile: Mutex<MyProfile>,
    port: u16,
    announce: Arc<Mutex<Announce>>,
    peers: Mutex<HashMap<String, Peer>>,
    conns: Mutex<HashMap<String, mpsc::Sender<Frame>>>,
    connecting: Mutex<HashSet<String>>,
    events: broadcast::Sender<Event>,
    files_dir: PathBuf,
    transfers: FileTransfer,
    /// offer yang sedang menunggu jawaban peer: msg_id → (accept_tx, done_tx)
    pending_accept: Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>,
    pending_done: Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>,
    tls: Tls,
    identity: Identity,
    allow_plaintext: bool,
}

impl MyProfile {
    /// Muat `profile.json`; kalau belum ada, pakai nama dari config.
    fn load(dir: &Path, identity: &Identity) -> Self {
        let path = dir.join("profile.json");
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                let name = v
                    .get("name")
                    .and_then(|x| x.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or(&identity.name)
                    .to_string();
                let avatar = v
                    .get("avatar")
                    .and_then(|x| x.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                let fp = format!("{}#{}", name, &identity.id()[..4]);
                return Self { name, fp, avatar };
            }
        }
        let name = identity.name.clone();
        let fp = identity.fp();
        Self { name, fp, avatar: None }
    }

    fn save(&self, dir: &Path) -> Result<()> {
        let v = serde_json::json!({
            "name": self.name,
            "avatar": self.avatar.clone().unwrap_or_default(),
        });
        std::fs::write(dir.join("profile.json"), serde_json::to_vec_pretty(&v)?)?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct Node(Arc<Inner>);

impl Node {
    pub async fn start(cfg: Config) -> Result<Node> {
        let identity = Identity::load_or_create(&cfg.data_dir, &cfg.name)?;
        let listener = TcpListener::bind(("0.0.0.0", cfg.port))
            .await
            .with_context(|| format!("bind TCP port {}", cfg.port))?;
        let port = listener.local_addr()?.port();

        // Profil user (nama + avatar) disimpan terpisah dari identitas: kunci
        // Ed25519 tidak berubah, tapi nama tampilan boleh diganti kapan saja.
        let profile = MyProfile::load(&cfg.data_dir, &identity);
        let announce = Arc::new(Mutex::new(Announce {
            v: PROTO_VERSION,
            id: identity.id(),
            name: profile.name.clone(),
            fp: profile.fp.clone(),
            os: std::env::consts::OS.to_string(),
            port,
            status: Status::Online,
            ts: now_ms(),
        }));

        let tls = Tls::init(&cfg.data_dir)?;

        let (events, _) = broadcast::channel(512);
        let node = Node(Arc::new(Inner {
            id: identity.id(),
            data_dir: cfg.data_dir.clone(),
            profile: Mutex::new(profile),
            port,
            announce: announce.clone(),
            peers: Mutex::new(HashMap::new()),
            conns: Mutex::new(HashMap::new()),
            connecting: Mutex::new(HashSet::new()),
            events,
            files_dir: cfg.data_dir.join("files"),
            transfers: FileTransfer::new(),
            pending_accept: Mutex::new(HashMap::new()),
            pending_done: Mutex::new(HashMap::new()),
            tls,
            identity,
            allow_plaintext: cfg.allow_plaintext,
        }));

        node.clone().spawn_acceptor(listener);
        node.clone().spawn_sweeper();
        if cfg.discovery {
            node.clone().spawn_discovery(announce, cfg.seeds)?;
        }

        Ok(node)
    }

    pub fn id(&self) -> &str {
        &self.0.id
    }

    pub fn name(&self) -> String {
        self.0.profile.lock().unwrap().name.clone()
    }

    pub fn fp(&self) -> String {
        self.0.profile.lock().unwrap().fp.clone()
    }

    /// Avatar sendiri (PNG base64), kalau ada.
    pub fn avatar(&self) -> Option<String> {
        self.0.profile.lock().unwrap().avatar.clone()
    }

    pub fn port(&self) -> u16 {
        self.0.port
    }

    pub fn status(&self) -> Status {
        self.0.announce.lock().unwrap().status
    }

    /// Folder tempat file masuk disimpan.
    pub fn files_dir(&self) -> PathBuf {
        self.0.files_dir.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.0.events.subscribe()
    }

    pub fn peers(&self) -> Vec<Peer> {
        let mut v: Vec<Peer> = self.0.peers.lock().unwrap().values().cloned().collect();
        v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        v
    }

    pub fn peer(&self, id: &str) -> Option<Peer> {
        self.0.peers.lock().unwrap().get(id).cloned()
    }

    pub fn is_connected(&self, peer_id: &str) -> bool {
        self.0.conns.lock().unwrap().contains_key(peer_id)
    }

    /// Cari peer berdasarkan prefix id / nama (buat CLI).
    pub fn find_peer(&self, needle: &str) -> Option<Peer> {
        let needle = needle.to_lowercase();
        let peers = self.peers();
        peers
            .iter()
            .find(|p| p.id.starts_with(&needle) || p.name.to_lowercase() == needle)
            .or_else(|| {
                peers
                    .iter()
                    .find(|p| p.name.to_lowercase().starts_with(&needle))
            })
            .cloned()
    }

    /// Ubah status presence sendiri dan sebarkan langsung ke peer yang terhubung.
    pub fn set_status(&self, status: Status) {
        self.0.announce.lock().unwrap().status = status;
        let frame = Frame::Presence {
            id: self.0.id.clone(),
            status,
        };
        for tx in self.0.conns.lock().unwrap().values() {
            let _ = tx.try_send(frame.clone());
        }
    }

    /// Kirim teks ke satu peer, atau [`BROADCAST`].
    pub fn send_text(&self, to: &str, body: &str) -> Result<String> {
        let (msg_id, ts) = self.dispatch(vec![to.to_string()], to.to_string(), body, None)?;
        self.emit(Event::Text {
            id: msg_id.clone(),
            from: self.0.id.clone(),
            from_name: self.name(),
            to: to.to_string(),
            body: body.to_string(),
            ts,
            mine: true,
            members: None,
        });
        Ok(msg_id)
    }

    /// Kirim pesan ke grup. `members` harus memuat id diri sendiri.
    /// Mengembalikan `(room_id, msg_id)`.
    pub fn send_group(&self, members: &[String], body: &str) -> Result<(String, String)> {
        let room = room_id(members);
        let (msg_id, ts) = self.dispatch(members.to_vec(), room.clone(), body, Some(members.to_vec()))?;
        self.emit(Event::Text {
            id: msg_id.clone(),
            from: self.0.id.clone(),
            from_name: self.name(),
            to: room.clone(),
            body: body.to_string(),
            ts,
            mine: true,
            members: Some(members.to_vec()),
        });
        let _ = ts;
        Ok((room, msg_id))
    }

    pub fn send_typing(&self, to: &str, on: bool) -> Result<()> {
        let frame = Frame::Typing {
            from: self.0.id.clone(),
            to: to.to_string(),
            on,
        };
        let conns = self.0.conns.lock().unwrap();
        let tx = conns
            .get(to)
            .cloned()
            .with_context(|| format!("tidak ada koneksi ke {to}"))?;
        drop(conns);
        let _ = tx.try_send(frame);
        Ok(())
    }

    /// Read receipt: "semua pesanmu sampai ts ini sudah kubaca".
    pub fn send_read(&self, to: &str, up_to_ts: u64) -> Result<()> {
        let frame = Frame::Read {
            from: self.0.id.clone(),
            to: to.to_string(),
            up_to_ts,
        };
        let conns = self.0.conns.lock().unwrap();
        let tx = conns
            .get(to)
            .cloned()
            .with_context(|| format!("tidak ada koneksi ke {to}"))?;
        drop(conns);
        let _ = tx.try_send(frame);
        Ok(())
    }

    /// Kirim file ke satu peer. Mengembalikan `msg_id`.
    ///
    /// Alur: offer → tunggu accept → stream chunk → tunggu done.
    /// Semua langkah punya timeout supaya UI tidak menggantung kalau peer diam.
    pub async fn send_file(&self, to: &str, path: &Path, timeout_secs: u64) -> Result<String> {
        let meta = FileTransfer::prepare(path)?;
        let tx = self
            .0
            .conns
            .lock()
            .unwrap()
            .get(to)
            .cloned()
            .with_context(|| format!("tidak ada koneksi ke {to}"))?;

        let msg_id = new_msg_id();
        let ts = now_ms();

        let (accept_tx, accept_rx) = tokio::sync::oneshot::channel();
        self.0
            .pending_accept
            .lock()
            .unwrap()
            .insert(msg_id.clone(), accept_tx);

        let offer = Frame::FileOffer {
            id: msg_id.clone(),
            from: self.0.id.clone(),
            from_name: self.name(),
            to: to.to_string(),
            name: meta.name.clone(),
            size: meta.size,
            sha256: meta.sha256.clone(),
            chunks: meta.chunks,
            ts,
            members: None,
        };
        if tx.send(offer).await.is_err() {
            self.0.pending_accept.lock().unwrap().remove(&msg_id);
            bail!("koneksi ke {to} tertutup sebelum file ditawarkan");
        }

        // tunggu keputusan peer
        let accepted = match tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            accept_rx,
        )
        .await
        {
            Ok(Ok(v)) => v,
            Ok(Err(_)) => bail!("transfer dibatalkan sebelum peer menjawab"),
            Err(_) => {
                self.0.pending_accept.lock().unwrap().remove(&msg_id);
                bail!("peer tidak menjawab tawaran file dalam {timeout_secs} detik");
            }
        };
        if !accepted {
            self.emit(Event::FileFailed {
                id: msg_id.clone(),
                name: meta.name.clone(),
                reason: "peer menolak file".into(),
            });
            bail!("peer menolak file");
        }

        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        self.0
            .pending_done
            .lock()
            .unwrap()
            .insert(msg_id.clone(), done_tx);

        self.emit(Event::FileOffer {
            id: msg_id.clone(),
            from: self.0.id.clone(),
            from_name: self.name(),
            to: to.to_string(),
            name: meta.name.clone(),
            size: meta.size,
            ts,
            mine: true,
            members: None,
        });

        // stream: kirim potongan lewat kanal writer koneksi
        let msg_id_clone = msg_id.clone();
        let transfers = self.0.transfers.clone();
        let node = self.clone();
        let path_owned = path.to_path_buf();
        let meta_for_task = meta.clone();
        let tx_for_task = tx.clone();
        let result = tokio::task::spawn_blocking(move || {
            transfers.stream_file(&msg_id_clone, &path_owned, &meta_for_task, |f| {
                tx_for_task.try_send(f).is_ok()
            })
        })
        .await
        .context("task pengiriman file gagal dijalankan")?;

        if let Err(e) = result {
            self.0.pending_done.lock().unwrap().remove(&msg_id);
            self.0.transfers.abort_inbound(&msg_id);
            self.emit(Event::FileFailed {
                id: msg_id.clone(),
                name: meta.name.clone(),
                reason: e.to_string(),
            });
            return Err(e);
        }

        // progres akhir
        self.emit(Event::FileProgress {
            id: msg_id.clone(),
            name: meta.name.clone(),
            sent_bytes: meta.size,
            total_bytes: meta.size,
            outgoing: true,
        });

        let ok = match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), done_rx)
            .await
        {
            Ok(Ok(v)) => v,
            Ok(Err(_)) => false,
            Err(_) => false,
        };
        self.0.transfers.finish_outgoing(&msg_id);
        if !ok {
            let reason = node
                .0
                .transfers
                .outgoing_progress(&msg_id)
                .map(|_| "penerima tidak mengonfirmasi file utuh")
                .unwrap_or("penerima tidak mengonfirmasi file utuh");
            self.emit(Event::FileFailed {
                id: msg_id.clone(),
                name: meta.name.clone(),
                reason: reason.to_string(),
            });
            bail!("{reason}");
        }
        Ok(msg_id)
    }

    /// Kirim ajakan voice call (SDP offer) ke satu peer.
    pub fn call_offer(&self, to: &str, call_id: &str, sdp: &str) -> Result<()> {
        self.call_offer_group(to, call_id, sdp, &[])
    }

    /// Sama seperti [`Node::call_offer`], tapi menyertakan daftar peserta grup.
    ///
    /// `members` kosong = panggilan 1:1. Penerima memakai daftar ini untuk
    /// menyambung ke peserta lain (mesh), bukan cuma ke pemanggil.
    pub fn call_offer_group(
        &self,
        to: &str,
        call_id: &str,
        sdp: &str,
        members: &[String],
    ) -> Result<()> {
        self.send_to(
            to,
            Frame::CallOffer {
                call_id: call_id.to_string(),
                from: self.0.id.clone(),
                from_name: self.name(),
                to: to.to_string(),
                sdp: sdp.to_string(),
                ts: now_ms(),
                members: members.to_vec(),
            },
        )
    }

    /// Jawab ajakan: `accept=false` → tolak.
    pub fn call_answer(&self, to: &str, call_id: &str, accept: bool, sdp: &str, reason: &str) -> Result<()> {
        self.send_to(
            to,
            Frame::CallAnswer {
                call_id: call_id.to_string(),
                from: self.0.id.clone(),
                accept,
                sdp: sdp.to_string(),
                reason: reason.to_string(),
            },
        )
    }

    /// Kirim kandidat ICE.
    pub fn call_ice(
        &self,
        to: &str,
        call_id: &str,
        candidate: &str,
        sdp_mid: &str,
        sdp_mline_index: u32,
    ) -> Result<()> {
        self.send_to(
            to,
            Frame::CallIce {
                call_id: call_id.to_string(),
                from: self.0.id.clone(),
                candidate: candidate.to_string(),
                sdp_mid: sdp_mid.to_string(),
                sdp_mline_index,
            },
        )
    }

    /// Tutup panggilan.
    pub fn call_end(&self, to: &str, call_id: &str, reason: &str) -> Result<()> {
        self.send_to(
            to,
            Frame::CallEnd {
                call_id: call_id.to_string(),
                from: self.0.id.clone(),
                reason: reason.to_string(),
            },
        )
    }

    /// Kirim satu frame ke satu peer.
    fn send_to(&self, to: &str, frame: Frame) -> Result<()> {
        let tx = self
            .0
            .conns
            .lock()
            .unwrap()
            .get(to)
            .cloned()
            .with_context(|| format!("tidak ada koneksi ke {to}"))?;
        tx.try_send(frame)
            .map_err(|_| anyhow::anyhow!("koneksi ke {to} sedang penuh"))
    }

    /// Kirim frame teks ke daftar target (peer id atau `*`).
    fn dispatch(
        &self,
        targets: Vec<String>,
        to_field: String,
        body: &str,
        members: Option<Vec<String>>,
    ) -> Result<(String, u64)> {
        let msg_id = new_msg_id();
        let ts = now_ms();
        let frame = Frame::Text {
            id: msg_id.clone(),
            from: self.0.id.clone(),
            from_name: self.name(),
            to: to_field,
            ts,
            body: body.to_string(),
            members,
        };
        let senders: Vec<mpsc::Sender<Frame>> = {
            let conns = self.0.conns.lock().unwrap();
            if targets.len() == 1 && targets[0] == BROADCAST {
                conns.values().cloned().collect()
            } else {
                targets
                    .iter()
                    .filter(|t| *t != &self.0.id)
                    .filter_map(|t| conns.get(t).cloned())
                    .collect()
            }
        };
        if senders.is_empty() {
            bail!("belum ada koneksi ke tujuan — peer belum ketemu atau belum connect");
        }
        for tx in senders {
            let _ = tx.try_send(frame.clone());
        }
        Ok((msg_id, ts))
    }

    /// Ubah nama tampilan dan/atau avatar, simpan, lalu beri tahu semua peer.
    ///
    /// `avatar = None` artinya tidak diubah. Untuk menghapus avatar, kirim
    /// `Some(String::new())`.
    pub fn set_profile(&self, name: Option<&str>, avatar: Option<String>) -> Result<()> {
        let (name, fp, av) = {
            let mut p = self.0.profile.lock().unwrap();
            if let Some(n) = name {
                let n = n.trim();
                if n.is_empty() {
                    bail!("nama tidak boleh kosong");
                }
                if n.chars().count() > 32 {
                    bail!("nama maksimal 32 karakter");
                }
                p.name = n.to_string();
                p.fp = format!("{}#{}", p.name, &self.0.id[..4]);
            }
            if let Some(a) = avatar {
                p.avatar = if a.is_empty() { None } else { Some(a) };
            }
            let snapshot = (p.name.clone(), p.fp.clone(), p.avatar.clone());
            let _ = p.save(&self.0.data_dir);
            snapshot
        };

        // Announce berikutnya membawa nama baru; update sekarang juga supaya
        // peer yang mendengarkan multicast langsung melihatnya.
        {
            let mut a = self.0.announce.lock().unwrap();
            a.name = name.clone();
            a.fp = fp.clone();
            a.ts = now_ms();
        }

        let frame = Frame::Profile {
            id: self.0.id.clone(),
            name,
            avatar: av.unwrap_or_default(),
            ts: now_ms(),
        };
        let senders: Vec<mpsc::Sender<Frame>> = self.0.conns.lock().unwrap().values().cloned().collect();
        for tx in senders {
            let _ = tx.try_send(frame.clone());
        }
        Ok(())
    }

    /// Dial peer secara manual (fallback kalau discovery diblok jaringan, dan buat test).
    pub async fn connect_peer(&self, addr: SocketAddr) -> Result<()> {
        let stream = TcpStream::connect(addr)
            .await
            .with_context(|| format!("dial {addr}"))?;
        self.handle_conn(stream, addr, true).await
    }

    fn emit(&self, e: Event) {
        let _ = self.0.events.send(e);
    }

    // ---------- internal ----------

    fn spawn_acceptor(self, listener: TcpListener) {
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, addr)) => {
                        let n = self.clone();
                        tokio::spawn(async move {
                            if let Err(e) = n.handle_conn(stream, addr, false).await {
                                tracing::debug!("koneksi masuk {addr} berakhir: {e:#}");
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!("accept: {e}");
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                }
            }
        });
    }

    fn spawn_sweeper(self) {
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(5));
            loop {
                tick.tick().await;
                let now = now_ms();
                let dead: Vec<String> = {
                    let peers = self.0.peers.lock().unwrap();
                    peers
                        .values()
                        .filter(|p| {
                            p.online && now.saturating_sub(p.last_seen) > PEER_TTL_SECS as u64 * 1000
                        })
                        .map(|p| p.id.clone())
                        .collect()
                };
                for id in dead {
                    {
                        let mut peers = self.0.peers.lock().unwrap();
                        if let Some(p) = peers.get_mut(&id) {
                            p.online = false;
                            p.status = Status::Online;
                        }
                    }
                    self.0.conns.lock().unwrap().remove(&id);
                    self.emit(Event::PeerDown(id));
                }
            }
        });
    }

    fn spawn_discovery(self, announce: Arc<Mutex<Announce>>, seeds: Vec<SocketAddr>) -> Result<()> {
        let (tx, mut rx) = mpsc::channel::<Seen>(64);
        discovery::spawn(announce, seeds, tx)?;

        tokio::spawn(async move {
            while let Some(seen) = rx.recv().await {
                self.on_announce(seen);
            }
        });
        Ok(())
    }

    fn on_announce(&self, seen: Seen) {
        let a = seen.announce;
        if a.id == self.0.id {
            return;
        }
        let addr = SocketAddr::new(seen.from, a.port);
        let peer = Peer {
            id: a.id.clone(),
            name: a.name.clone(),
            fp: a.fp.clone(),
            os: a.os.clone(),
            addr,
            last_seen: now_ms(),
            status: a.status,
            online: true,
            // Dari announce multicast: belum ada bukti kriptografis.
            verified: false,
            // Announce tidak membawa avatar; diisi nanti oleh frame Profile.
            avatar: None,
        };
        let (is_new, status_changed) = self.upsert_peer(peer.clone());
        if is_new {
            self.emit(Event::PeerUp(peer.clone()));
        } else if status_changed {
            self.emit(Event::PeerStatus {
                id: peer.id.clone(),
                status: peer.status,
            });
        }
        self.maybe_connect(&peer);
    }

    /// Insert/refresh peer. `(baru, status berubah)`.
    /// Terapkan nama/avatar peer yang baru tanpa menghapus status lain.
    fn apply_peer_profile(&self, id: &str, name: &str, avatar: Option<String>) {
        let mut peers = self.0.peers.lock().unwrap();
        if let Some(p) = peers.get_mut(id) {
            p.name = name.to_string();
            p.fp = format!("{}#{}", name, &id[..4.min(id.len())]);
            if avatar.is_some() {
                p.avatar = avatar;
            }
        }
    }

    fn upsert_peer(&self, p: Peer) -> (bool, bool) {
        let mut peers = self.0.peers.lock().unwrap();
        match peers.get_mut(&p.id) {
            Some(old) => {
                let was_online = old.online;
                let status_changed = old.status != p.status || !was_online;
                old.last_seen = p.last_seen;
                old.online = true;
                old.status = p.status;
                if !p.name.is_empty() {
                    old.name = p.name;
                    old.fp = p.fp;
                }
                if p.os != "?" {
                    old.os = p.os;
                }
                old.addr = SocketAddr::new(p.addr.ip(), p.addr.port());
                // `verified` hanya boleh naik, tidak boleh turun: announce multicast
                // tidak membawa bukti, sedangkan handshake TLS membawanya. Kalau
                // announce datang belakangan, status terverifikasi tidak boleh hilang.
                old.verified = old.verified || p.verified;
                // Avatar dari frame Profile tidak boleh hilang karena announce
                // berikutnya (announce tidak membawa avatar).
                if p.avatar.is_some() {
                    old.avatar = p.avatar.clone();
                }
                (false, status_changed)
            }
            None => {
                peers.insert(p.id.clone(), p);
                (true, false)
            }
        }
    }

    fn maybe_connect(&self, p: &Peer) {
        if self.0.conns.lock().unwrap().contains_key(&p.id) {
            return;
        }
        // hanya sisi dengan id lebih kecil yang melakukan dial
        if self.0.id.as_str() >= p.id.as_str() {
            return;
        }
        if !self.0.connecting.lock().unwrap().insert(p.id.clone()) {
            return;
        }
        let node = self.clone();
        let pid = p.id.clone();
        let addr = p.addr;
        tokio::spawn(async move {
            match TcpStream::connect(addr).await {
                Ok(stream) => {
                    if let Err(e) = node.handle_conn(stream, addr, true).await {
                        tracing::debug!("koneksi keluar {addr} berakhir: {e:#}");
                    }
                }
                Err(e) => tracing::debug!("gagal dial {addr}: {e}"),
            }
            node.0.connecting.lock().unwrap().remove(&pid);
        });
    }

    async fn handle_conn(&self, stream: TcpStream, addr: SocketAddr, outbound: bool) -> Result<()> {
        let _ = stream.set_nodelay(true);
        let tls = self.0.tls.clone();

        // Handshake TLS dulu: setelah ini seluruh frame (termasuk file) terenkripsi.
        // `exporter` adalah material rahasia sesi; dipakai mengikat tanda tangan hello
        // ke koneksi ini sehingga tanda tangan tidak bisa dipakai ulang di sesi lain.
        if outbound {
            let s = tls.connect(stream).await?;
            let ex = crate::tls::exporter_client(s.get_ref().1)?;
            self.run_conn(s, addr, true, ex).await
        } else {
            let s = tls.accept(stream).await?;
            let ex = crate::tls::exporter_server(s.get_ref().1)?;
            self.run_conn(s, addr, false, ex).await
        }
    }

    /// Bagian protokol setelah TLS siap: tukar hello + auth, lalu loop frame.
    async fn run_conn<S>(&self, s: S, addr: SocketAddr, outbound: bool, exporter: [u8; 32]) -> Result<()>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
    {
        if outbound {
            tracing::debug!("dial keluar ke {addr} (TLS)");
        }

        let (mut rd, mut wr) = tokio::io::split(s);

        // 1. Tukar Hello (belum bertanda tangan — id peer belum diketahui).
        let hello = Frame::Hello {
            v: PROTO_VERSION,
            id: self.0.id.clone(),
            name: self.name(),
            fp: self.fp(),
            port: self.0.port,
            os: std::env::consts::OS.to_string(),
            status: self.status(),
            sig: String::new(),
            pk: String::new(),
        };
        write_frame(&mut wr, &hello).await.context("kirim hello")?;

        let (pid, pname, pfp, pport, pos, pstatus) =
            match read_frame(&mut rd).await.context("baca hello")? {
                Some(Frame::Hello { id, name, fp, port, os, status, .. }) => {
                    (id, name, fp, port, os, status)
                }
                Some(f) => bail!("frame pertama bukan hello: {f:?}"),
                None => return Ok(()),
            };
        if pid == self.0.id {
            bail!("menerima koneksi dari diri sendiri");
        }

        // 2. Tukar Auth: tanda tangan atas exporter sesi + kedua id.
        let my_sig = self.0.identity.sign_hex(&crate::tls::hello_msg(&exporter, &self.0.id, &pid));
        write_frame(
            &mut wr,
            &Frame::Auth { pk: self.0.identity.pk_hex(), sig: my_sig },
        )
        .await
        .context("kirim auth")?;

        let verified = match read_frame(&mut rd).await.context("baca auth")? {
            Some(Frame::Auth { pk, sig }) => {
                let basis = crate::tls::hello_msg(&exporter, &self.0.id, &pid);
                crate::identity::Identity::verify_hex(&pk, &basis, &sig, Some(&pid))
                    .with_context(|| format!("verifikasi identitas {pname} ({pid})"))?;
                true
            }
            // Peer lama: tidak bisa membuktikan identitas.
            Some(other) => {
                if !self.0.allow_plaintext {
                    bail!("peer {pname} ({pid}) tidak mendukung autentikasi TLS — koneksi ditolak (frame: {other:?})");
                }
                self.emit(Event::Warn(format!(
                    "{pname} terhubung tanpa verifikasi identitas — isi chat tidak dijamin aman"
                )));
                // Frame ini bukan Auth, jadi harus tetap diproses sebagai frame biasa.
                self.handle_frame(other).await.ok();
                false
            }
            None => return Ok(()),
        };

        // 3. Tukar profil (nama + avatar) supaya peer langsung punya data terbaru.
        {
            let (my_name, my_avatar) = {
                let p = self.0.profile.lock().unwrap();
                (p.name.clone(), p.avatar.clone().unwrap_or_default())
            };
            let _ = write_frame(
                &mut wr,
                &Frame::Profile {
                    id: self.0.id.clone(),
                    name: my_name,
                    avatar: my_avatar,
                    ts: now_ms(),
                },
            )
            .await;
        }
        match read_frame(&mut rd).await {
            Ok(Some(Frame::Profile { id, name, avatar, .. })) if id == pid => {
                if !name.trim().is_empty() {
                    let av = if avatar.is_empty() { None } else { Some(avatar) };
                    self.apply_peer_profile(&pid, &name, av.clone());
                    self.emit(Event::PeerProfile { id: pid.clone(), name, avatar: av });
                }
            }
            // Peer lama tidak mengirim profil: bukan masalah, nama dari Hello dipakai.
            Ok(Some(other)) => {
                self.handle_frame(other).await.ok();
            }
            Ok(None) => return Ok(()),
            Err(_) => {}
        }

        let (tx, mut rx) = mpsc::channel::<Frame>(256);
        if self.0.conns.lock().unwrap().insert(pid.clone(), tx).is_some() {
            tracing::debug!("koneksi lama ke {pid} diganti yang baru");
        }
        self.0.connecting.lock().unwrap().remove(&pid);

        // `wr` (WriteHalf) dipindah ke task penulis; `rd` tetap di sini.
        let writer = tokio::spawn(async move {
            while let Some(f) = rx.recv().await {
                if write_frame(&mut wr, &f).await.is_err() {
                    break;
                }
            }
        });

        let peer = Peer {
            id: pid.clone(),
            name: pname,
            fp: pfp,
            os: if pos.is_empty() { "?".to_string() } else { pos },
            addr: SocketAddr::new(addr.ip(), pport),
            last_seen: now_ms(),
            status: pstatus,
            online: true,
            verified,
            // Diisi dari frame Profile yang ditukar setelah Auth.
            avatar: None,
        };
        let (is_new, status_changed) = self.upsert_peer(peer.clone());
        if is_new {
            self.emit(Event::PeerUp(peer.clone()));
        } else if status_changed {
            self.emit(Event::PeerStatus { id: peer.id.clone(), status: peer.status });
        }

        let res = loop {
            match read_frame(&mut rd).await {
                Ok(Some(f)) => {
                    if let Err(e) = self.handle_frame(f).await {
                        tracing::debug!("handle_frame: {e:#}");
                    }
                }
                Ok(None) => break Ok(()),
                Err(e) => break Err(e),
            }
        };

        writer.abort();
        self.remove_conn(&pid);
        res
    }

    async fn handle_frame(&self, f: Frame) -> Result<()> {
        match f {
            Frame::Hello { .. } => {}
            // Auth hanya diproses di fase handshake (run_conn). Kalau muncul lagi
            // di tengah sesi, abaikan: identitas sudah ditetapkan saat koneksi dibuka.
            Frame::Auth { .. } => {}
            Frame::Profile { id, name, avatar, .. } => {
                // Hanya terima profil dari peer yang benar-benar terhubung, dan
                // jangan biarkan id lain menimpa profil orang lain.
                let known = self.0.conns.lock().unwrap().contains_key(&id);
                if known && !name.trim().is_empty() {
                    let av = if avatar.is_empty() { None } else { Some(avatar) };
                    self.apply_peer_profile(&id, &name, av.clone());
                    self.emit(Event::PeerProfile { id, name, avatar: av });
                }
            }
            Frame::Text {
                id,
                from,
                from_name,
                to,
                ts,
                body,
                members,
            } => {
                let for_me = to == BROADCAST
                    || to == self.0.id
                    || (is_room(&to)
                        && members
                            .as_ref()
                            .map(|m| m.iter().any(|x| x == &self.0.id))
                            .unwrap_or(false));
                if !for_me {
                    let _ = (id, from, from_name, ts, body, members);
                    return Ok(());
                }
                if from != self.0.id {
                    if let Some(p) = self.0.peers.lock().unwrap().get_mut(&from) {
                        p.last_seen = now_ms();
                        p.online = true;
                    }
                }
                self.emit(Event::Text {
                    id: id.clone(),
                    from: from.clone(),
                    from_name,
                    to,
                    body,
                    ts,
                    mine: false,
                    members,
                });
                let ack = Frame::Ack { id, ts: now_ms() };
                if let Some(tx) = self.0.conns.lock().unwrap().get(&from).cloned() {
                    let _ = tx.try_send(ack);
                }
            }
            Frame::Ack { id, ts } => self.emit(Event::Ack { id, ts }),
            Frame::Typing { from, to, on } => self.emit(Event::Typing { from, to, on }),
            Frame::Read {
                from,
                to,
                up_to_ts,
            } => self.emit(Event::Read {
                from,
                to,
                up_to_ts,
            }),
            Frame::FileOffer {
                id,
                from,
                from_name,
                to,
                name,
                size,
                sha256,
                chunks,
                ts,
                members,
            } => {
                // Terima otomatis: file dari peer yang terhubung, ukuran wajar.
                // Penolakan manual bisa ditambahkan nanti lewat command UI.
                let res = self.0.transfers.begin_inbound_ctx(
                    &self.0.files_dir,
                    &id,
                    &name,
                    size,
                    &sha256,
                    chunks,
                    crate::files::FileContext {
                        from: from.clone(),
                        from_name: from_name.clone(),
                        to: self.0.id.clone(),
                        members: members.clone(),
                        ts,
                    },
                );
                let accept = match res {
                    Ok(_) => true,
                    Err(e) => {
                        self.emit(Event::FileFailed {
                            id: id.clone(),
                            name: name.clone(),
                            reason: e.to_string(),
                        });
                        false
                    }
                };
                // guard diambil & dilepas sebelum await (kalau tidak, future jadi !Send)
                let sender = self.0.conns.lock().unwrap().get(&from).cloned();
                if let Some(tx) = sender {
                    let _ = tx
                        .send(Frame::FileAccept {
                            id: id.clone(),
                            accept,
                            reason: if accept {
                                String::new()
                            } else {
                                "penerima gagal menyiapkan file".into()
                            },
                        })
                        .await;
                }
                if accept {
                    self.emit(Event::FileOffer {
                        id,
                        from,
                        from_name,
                        to,
                        name,
                        size,
                        ts,
                        mine: false,
                        members,
                    });
                }
            }
            Frame::FileChunk { id, idx, data } => {
                match self.0.transfers.write_chunk(&id, idx, &data) {
                    Ok(Some((path, ctx))) => {
                        // konfirmasi dikirim ke pengirim yang benar, bukan sembarang koneksi
                        if let Some(tx) = self.0.conns.lock().unwrap().get(&ctx.from).cloned() {
                            let _ = tx.try_send(Frame::FileDone {
                                id: id.clone(),
                                ok: true,
                                reason: String::new(),
                            });
                        }
                        let name = path
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_default();
                        // buang prefix "<msg_id>-" yang dipakai supaya nama file unik
                        let display = name
                            .strip_prefix(&format!("{id}-"))
                            .unwrap_or(&name)
                            .to_string();
                        self.emit(Event::FileReceived {
                            id,
                            from: ctx.from,
                            from_name: ctx.from_name,
                            to: ctx.to,
                            name: display,
                            size: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
                            path: path.to_string_lossy().to_string(),
                            ts: ctx.ts.max(now_ms()),
                            members: ctx.members,
                        });
                    }
                    Ok(None) => {
                        if let Some(t) = self.0.transfers.incoming_progress(&id) {
                            self.emit(Event::FileProgress {
                                id: id.clone(),
                                name: t.name,
                                sent_bytes: t.received_bytes,
                                total_bytes: t.size,
                                outgoing: false,
                            });
                        }
                    }
                    Err(e) => {
                        // beri tahu pengirim bahwa file gagal & sudah dihapus
                        if let Some(ctx_peer) = self.0.transfers.incoming_sender(&id) {
                            if let Some(tx) = self.0.conns.lock().unwrap().get(&ctx_peer).cloned() {
                                let _ = tx.try_send(Frame::FileDone {
                                    id: id.clone(),
                                    ok: false,
                                    reason: e.to_string(),
                                });
                            }
                        }
                        self.0.transfers.abort_inbound(&id);
                        self.emit(Event::FileFailed {
                            id: id.clone(),
                            name: String::new(),
                            reason: e.to_string(),
                        });
                    }
                }
            }
            Frame::FileAccept { id, accept, .. } => {
                if let Some(tx) = self.0.pending_accept.lock().unwrap().remove(&id) {
                    let _ = tx.send(accept);
                }
            }
            Frame::FileDone { id, ok, .. } => {
                if let Some(tx) = self.0.pending_done.lock().unwrap().remove(&id) {
                    let _ = tx.send(ok);
                }
            }
            Frame::CallOffer {
                call_id,
                from,
                from_name,
                sdp,
                members,
                ..
            } => {
                self.emit(Event::CallIncoming {
                    call_id,
                    from,
                    from_name,
                    sdp,
                    members,
                });
            }
            Frame::CallAnswer {
                call_id,
                from,
                accept,
                sdp,
                reason,
            } => {
                if accept {
                    self.emit(Event::CallAccepted {
                        call_id,
                        from,
                        sdp,
                    });
                } else {
                    self.emit(Event::CallRejected {
                        call_id,
                        from,
                        reason,
                    });
                }
            }
            Frame::CallIce {
                call_id,
                from,
                candidate,
                sdp_mid,
                sdp_mline_index,
            } => {
                self.emit(Event::CallIce {
                    call_id,
                    from,
                    candidate,
                    sdp_mid,
                    sdp_mline_index,
                });
            }
            Frame::CallEnd {
                call_id,
                from,
                reason,
            } => {
                self.emit(Event::CallEnded {
                    call_id,
                    from,
                    reason,
                });
            }
            Frame::Presence { id, status } => {
                if let Some(p) = self.0.peers.lock().unwrap().get_mut(&id) {
                    p.status = status;
                    p.last_seen = now_ms();
                }
                self.emit(Event::PeerStatus { id, status });
            }
        }
        Ok(())
    }

    fn remove_conn(&self, peer_id: &str) {
        self.0.conns.lock().unwrap().remove(peer_id);
    }
}