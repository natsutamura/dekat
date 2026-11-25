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
}

impl Config {
    pub fn new(name: impl Into<String>, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            port: DEFAULT_APP_PORT,
            data_dir: data_dir.into(),
            seeds: Vec::new(),
            discovery: true,
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
    Warn(String),
}

struct Inner {
    id: String,
    name: String,
    fp: String,
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

        let announce = Arc::new(Mutex::new(Announce {
            v: PROTO_VERSION,
            id: identity.id(),
            name: identity.name.clone(),
            fp: identity.fp(),
            os: std::env::consts::OS.to_string(),
            port,
            status: Status::Online,
            ts: now_ms(),
        }));

        let (events, _) = broadcast::channel(512);
        let node = Node(Arc::new(Inner {
            id: identity.id(),
            name: identity.name.clone(),
            fp: identity.fp(),
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

    pub fn name(&self) -> &str {
        &self.0.name
    }

    pub fn fp(&self) -> &str {
        &self.0.fp
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
            from_name: self.0.name.clone(),
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
            from_name: self.0.name.clone(),
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
            from_name: self.0.name.clone(),
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
            from_name: self.0.name.clone(),
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
            from_name: self.0.name.clone(),
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
        let (mut rd, mut wr) = stream.into_split();

        if outbound {
            tracing::debug!("dial keluar ke {addr}");
        }

        // Hello dikirim dua arah (dialer maupun penerima) — tanpa ini sisi
        // inbound akan menunggu selamanya dan koneksi tidak pernah siap.
        let hello = Frame::Hello {
            v: PROTO_VERSION,
            id: self.0.id.clone(),
            name: self.0.name.clone(),
            fp: self.0.fp.clone(),
            port: self.0.port,
            os: std::env::consts::OS.to_string(),
            status: self.status(),
        };
        write_frame(&mut wr, &hello).await.context("kirim hello")?;

        let (pid, pname, pfp, pport, pos, pstatus) =
            match read_frame(&mut rd).await.context("baca hello")? {
                Some(Frame::Hello {
                    id,
                    name,
                    fp,
                    port,
                    os,
                    status,
                    ..
                }) => (id, name, fp, port, os, status),
                Some(f) => bail!("frame pertama bukan hello: {f:?}"),
                None => return Ok(()),
            };
        if pid == self.0.id {
            bail!("menerima koneksi dari diri sendiri");
        }

        let (tx, mut rx) = mpsc::channel::<Frame>(256);
        let replaced = self.0.conns.lock().unwrap().insert(pid.clone(), tx);
        if replaced.is_some() {
            tracing::debug!("koneksi lama ke {pid} diganti yang baru");
        }
        self.0.connecting.lock().unwrap().remove(&pid);

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