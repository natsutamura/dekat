//! Node: gabungan discovery + transport + peer table + event stream.
//!
//! Aturan koneksi M0: untuk tiap pasangan node, hanya node dengan `id`
//! lebih kecil (lexicographic) yang melakukan dial. Hasilnya tepat satu
//! koneksi TCP bidirectional per pasangan — tidak ada pesan dobel.

use crate::{
    discovery::{self, Seen},
    identity::Identity,
    proto::*,
};
use anyhow::{bail, Context, Result};
use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::PathBuf,
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
}

impl Config {
    pub fn new(name: impl Into<String>, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            port: DEFAULT_APP_PORT,
            data_dir: data_dir.into(),
            seeds: Vec::new(),
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
}

#[derive(Clone, Debug)]
pub enum Event {
    PeerUp(Peer),
    PeerDown(String),
    Text {
        from: String,
        from_name: String,
        to: String,
        body: String,
        ts: u64,
        mine: bool,
    },
    Ack {
        id: String,
        ts: u64,
    },
    Warn(String),
}

struct Inner {
    id: String,
    name: String,
    fp: String,
    port: u16,
    peers: Mutex<HashMap<String, Peer>>,
    conns: Mutex<HashMap<String, mpsc::Sender<Frame>>>,
    connecting: Mutex<HashSet<String>>,
    events: broadcast::Sender<Event>,
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

        let (events, _) = broadcast::channel(512);
        let node = Node(Arc::new(Inner {
            id: identity.id(),
            name: identity.name.clone(),
            fp: identity.fp(),
            port,
            peers: Mutex::new(HashMap::new()),
            conns: Mutex::new(HashMap::new()),
            connecting: Mutex::new(HashSet::new()),
            events,
        }));

        node.clone().spawn_acceptor(listener);
        node.clone().spawn_sweeper();
        node.clone().spawn_discovery(cfg.seeds)?;

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

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.0.events.subscribe()
    }

    pub fn peers(&self) -> Vec<Peer> {
        let mut v: Vec<Peer> = self.0.peers.lock().unwrap().values().cloned().collect();
        v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        v
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

    /// Kirim teks. `to` = id peer, atau [`BROADCAST`] untuk semua koneksi.
    pub fn send_text(&self, to: &str, body: &str) -> Result<String> {
        let msg_id = new_msg_id();
        let ts = now_ms();
        let frame = Frame::Text {
            id: msg_id.clone(),
            from: self.0.id.clone(),
            from_name: self.0.name.clone(),
            to: to.to_string(),
            ts,
            body: body.to_string(),
        };
        let targets: Vec<mpsc::Sender<Frame>> = {
            let conns = self.0.conns.lock().unwrap();
            if to == BROADCAST {
                conns.values().cloned().collect()
            } else {
                conns.get(to).cloned().into_iter().collect()
            }
        };
        if targets.is_empty() {
            bail!("tidak ada koneksi ke '{to}' — peer belum ketemu atau belum connect");
        }
        for tx in targets {
            let _ = tx.try_send(frame.clone());
        }
        self.emit(Event::Text {
            from: self.0.id.clone(),
            from_name: self.0.name.clone(),
            to: to.to_string(),
            body: body.to_string(),
            ts,
            mine: true,
        });
        Ok(msg_id)
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
                        .filter(|p| now.saturating_sub(p.last_seen) > PEER_TTL_SECS * 1000)
                        .map(|p| p.id.clone())
                        .collect()
                };
                for id in dead {
                    self.0.peers.lock().unwrap().remove(&id);
                    self.0.conns.lock().unwrap().remove(&id);
                    self.emit(Event::PeerDown(id));
                }
            }
        });
    }

    fn spawn_discovery(self, seeds: Vec<SocketAddr>) -> Result<()> {
        let me = Announce {
            v: PROTO_VERSION,
            id: self.0.id.clone(),
            name: self.0.name.clone(),
            fp: self.0.fp.clone(),
            os: std::env::consts::OS.to_string(),
            port: self.0.port,
            ts: now_ms(),
        };
        let (tx, mut rx) = mpsc::channel::<Seen>(64);
        discovery::spawn(me, seeds, tx)?;

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
        };
        let is_new = self.upsert_peer(peer.clone());
        if is_new {
            self.emit(Event::PeerUp(peer.clone()));
        }
        self.maybe_connect(&peer);
    }

    /// Insert/refresh peer. `true` kalau peer baru.
    fn upsert_peer(&self, p: Peer) -> bool {
        let mut peers = self.0.peers.lock().unwrap();
        match peers.get_mut(&p.id) {
            Some(old) => {
                old.last_seen = p.last_seen;
                if !p.name.is_empty() {
                    old.name = p.name;
                    old.fp = p.fp;
                }
                if p.os != "?" {
                    old.os = p.os;
                }
                old.addr = SocketAddr::new(p.addr.ip(), p.addr.port());
                false
            }
            None => {
                peers.insert(p.id.clone(), p);
                true
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
        };
        write_frame(&mut wr, &hello).await.context("kirim hello")?;

        let (pid, pname, pfp, pport) = match read_frame(&mut rd).await.context("baca hello")? {
            Some(Frame::Hello {
                id, name, fp, port, ..
            }) => (id, name, fp, port),
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
            os: "?".to_string(),
            addr: SocketAddr::new(addr.ip(), pport),
            last_seen: now_ms(),
        };
        if self.upsert_peer(peer.clone()) {
            self.emit(Event::PeerUp(peer));
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
            } => {
                if to == BROADCAST || to == self.0.id {
                    if from != self.0.id {
                        // refresh presence: peer ini jelas hidup
                        if let Some(p) = self.0.peers.lock().unwrap().get_mut(&from) {
                            p.last_seen = now_ms();
                        }
                    }
                    self.emit(Event::Text {
                        from: from.clone(),
                        from_name,
                        to,
                        body,
                        ts,
                        mine: false,
                    });
                    let ack = Frame::Ack { id, ts: now_ms() };
                    if let Some(tx) = self.0.conns.lock().unwrap().get(&from).cloned() {
                        let _ = tx.try_send(ack);
                    }
                }
            }
            Frame::Ack { id, ts } => self.emit(Event::Ack { id, ts }),
        }
        Ok(())
    }

    fn remove_conn(&self, peer_id: &str) {
        self.0.conns.lock().unwrap().remove(peer_id);
    }
}
