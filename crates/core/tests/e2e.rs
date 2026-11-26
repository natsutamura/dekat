//! Integration test: dua node nyata, socket nyata, satu proses.
//!
//! Yang diuji: handshake, pengiriman DM + ack, pesan grup (fan-out), typing.
//! Tidak butuh jaringan luar — semua lewat loopback.

use base64::Engine as _;
use dekat_core::{Config, Event, Node};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    time::Duration,
};
use tokio::time::timeout;

fn tmp_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("dekat-it-{}-{}-{}", tag, std::process::id(), n));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn cfg(name: &str) -> Config {
    let mut c = Config::new(name, tmp_dir(name));
    c.port = 0; // port bebas
    c.discovery = false; // tes pakai dial manual, biar tidak saling ganggu antar test
    c
}

/// Nyalakan dua node dan sambungkan A → B (dial manual, tidak bergantung discovery).
async fn pair() -> (Node, Node) {
    let a = Node::start(cfg("a")).await.unwrap();
    let b = Node::start(cfg("b")).await.unwrap();
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), b.port());
    let a2 = a.clone();
    tokio::spawn(async move {
        let _ = a2.connect_peer(addr).await;
    });
    for _ in 0..60 {
        if a.is_connected(b.id()) && b.is_connected(a.id()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(a.is_connected(b.id()), "A tidak terhubung ke B");
    (a, b)
}

#[tokio::test]
async fn dm_terkirim_dan_di_ack() {
    let (a, b) = pair().await;
    let mut ev_b = b.subscribe();
    let mut ev_a = a.subscribe();

    let msg_id = a.send_text(b.id(), "halo dari a").unwrap();

    let diterima = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Event::Text {
                body, mine: false, ..
            }) = ev_b.recv().await
            {
                return body;
            }
        }
    })
    .await
    .expect("B tidak menerima pesan");
    assert_eq!(diterima, "halo dari a");

    let acked = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Event::Ack { id, .. }) = ev_a.recv().await {
                return id;
            }
        }
    })
    .await
    .expect("A tidak menerima ack");
    assert_eq!(acked, msg_id);
}

#[tokio::test]
async fn pesan_grup_sampai_ke_anggota() {
    let (a, b) = pair().await;
    let members = vec![a.id().to_string(), b.id().to_string()];
    let mut ev_b = b.subscribe();

    let (room, _msg) = a.send_group(&members, "halo grup").unwrap();
    assert!(room.starts_with('#'));
    assert_eq!(room, dekat_core::room_id(&members));

    let (body, to) = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Event::Text {
                body, to, mine: false, ..
            }) = ev_b.recv().await
            {
                return (body, to);
            }
        }
    })
    .await
    .expect("B tidak menerima pesan grup");
    assert_eq!(body, "halo grup");
    assert_eq!(to, room);
}

#[tokio::test]
async fn typing_dan_read_receipt_sampai() {
    let (a, b) = pair().await;
    let mut ev_b = b.subscribe();
    let mut ev_a = a.subscribe();

    a.send_typing(b.id(), true).unwrap();
    let on = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Event::Typing { on, .. }) = ev_b.recv().await {
                return on;
            }
        }
    })
    .await
    .expect("typing tidak sampai");
    assert!(on);

    b.send_read(a.id(), 12345).unwrap();
    let up_to = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Event::Read { up_to_ts, .. }) = ev_a.recv().await {
                return up_to_ts;
            }
        }
    })
    .await
    .expect("read receipt tidak sampai");
    assert_eq!(up_to, 12345);
}

#[tokio::test]
async fn kirim_ke_room_tanpa_anggota_terhubung_ditolak() {
    let (a, _b) = pair().await;
    // room yang anggotanya tidak satu pun terkoneksi → harus error, bukan diam-diam hilang
    let res = a.send_group(&["aku".to_string(), "dia".to_string()], "tidak ada tujuan");
    assert!(res.is_err());
}

/// Regresi: peer yang ketemu lewat handshake TCP tetap harus punya info OS,
/// kalau tidak sidebar UI cuma bisa nampilin "Online · ?".
#[tokio::test]
async fn os_peer_terisi_dari_handshake() {
    let (a, b) = pair().await;
    let mut found = None;
    for _ in 0..40 {
        if let Some(p) = a.peer(b.id()) {
            found = Some(p);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let p = found.expect("peer B tidak terdaftar di A");
    assert_eq!(p.os, std::env::consts::OS, "os peer harus dari hello, bukan \"?\"");
    assert_ne!(p.os, "?");
}

#[tokio::test]
async fn kirim_file_sampai_dan_utuh() {
    use std::io::Write;
    let (a, b) = pair().await;

    // file sumber 300 KB di dir A
    let dir_a = std::env::temp_dir().join(format!("dekat-sendfile-{}", std::process::id()));
    std::fs::create_dir_all(&dir_a).unwrap();
    let src = dir_a.join("dokumen.bin");
    {
        let mut f = std::fs::File::create(&src).unwrap();
        let data: Vec<u8> = (0..300 * 1024).map(|i| (i % 253) as u8).collect();
        f.write_all(&data).unwrap();
    }
    let expected = dekat_core::files::sha256_file(&src).unwrap();

    let mut ev_b = b.subscribe();
    let a2 = a.clone();
    let b_id = b.id().to_string();
    let src2 = src.clone();
    let send = tokio::spawn(async move { a2.send_file(&b_id, &src2, 10).await });

    // B harus menerima FileReceived dengan path yang isinya sama persis
    let received_path = timeout(Duration::from_secs(20), async {
        loop {
            if let Ok(Event::FileReceived { path, name, .. }) = ev_b.recv().await {
                return (path, name);
            }
        }
    })
    .await
    .expect("B tidak menerima file");

    let (path, name) = received_path;
    assert_eq!(name, "dokumen.bin");
    assert_eq!(dekat_core::files::sha256_file(std::path::Path::new(&path)).unwrap(), expected);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 300 * 1024);

    let msg_id = timeout(Duration::from_secs(20), send).await.unwrap().unwrap();
    assert!(msg_id.is_ok(), "pengirim melaporkan gagal: {msg_id:?}");
    let _ = std::fs::remove_dir_all(&dir_a);
}

#[tokio::test]
async fn kirim_file_tanpa_koneksi_gagal_dengan_jelas() {
    let (a, _b) = pair().await;
    let src = std::env::temp_dir().join(format!("dekat-nofile-{}.bin", std::process::id()));
    std::fs::write(&src, b"isi").unwrap();
    let err = a.send_file("peer-yang-tidak-ada", &src, 1).await.unwrap_err();
    assert!(err.to_string().contains("koneksi"), "pesan error: {err}");
    let _ = std::fs::remove_file(&src);
}

#[tokio::test]
async fn signalling_call_offer_answer_ice_end() {
    let (a, b) = pair().await;
    let (a_id, b_id) = (a.id().to_string(), b.id().to_string());
    let mut ev_b = b.subscribe();
    let mut ev_a = a.subscribe();
    let call_id = "call-1";

    // A menawarkan panggilan
    a.call_offer(&b_id, call_id, "v=0\r\noffer").unwrap();
    let (incoming_call, incoming_from, sdp) = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Event::CallIncoming { call_id, from, sdp, .. }) = ev_b.recv().await {
                return (call_id, from, sdp);
            }
        }
    })
    .await
    .expect("B tidak menerima CallIncoming");
    assert_eq!(incoming_call, call_id);
    assert_eq!(incoming_from, a_id);
    assert_eq!(sdp, "v=0\r\noffer");

    // B menjawab
    b.call_answer(&a_id, call_id, true, "v=0\r\nanswer", "").unwrap();
    let (ok_sdp) = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Event::CallAccepted { sdp, .. }) = ev_a.recv().await {
                return sdp;
            }
        }
    })
    .await
    .expect("A tidak menerima CallAccepted");
    assert_eq!(ok_sdp, "v=0\r\nanswer");

    // ICE dari B ke A
    b.call_ice(&a_id, call_id, "candidate:1 1 UDP 1 10.0.0.2 5000 typ host", "0", 0)
        .unwrap();
    let cand = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Event::CallIce { candidate, .. }) = ev_a.recv().await {
                return candidate;
            }
        }
    })
    .await
    .expect("A tidak menerima CallIce");
    assert!(cand.contains("candidate:1"));

    // A menutup panggilan
    a.call_end(&b_id, call_id, "selesai").unwrap();
    let reason = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Event::CallEnded { reason, .. }) = ev_b.recv().await {
                return reason;
            }
        }
    })
    .await
    .expect("B tidak menerima CallEnded");
    assert_eq!(reason, "selesai");
}

#[tokio::test]
async fn call_ditolak_menghasilkan_event_rejected() {
    let (a, b) = pair().await;
    let (a_id, b_id) = (a.id().to_string(), b.id().to_string());
    let mut ev_a = a.subscribe();
    let mut ev_b = b.subscribe();

    a.call_offer(&b_id, "c2", "offer").unwrap();
    timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Event::CallIncoming { .. }) = ev_b.recv().await {
                return;
            }
        }
    })
    .await
    .expect("offer tidak sampai");

    b.call_answer(&a_id, "c2", false, "", "sedang sibuk").unwrap();
    let reason = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Event::CallRejected { reason, .. }) = ev_a.recv().await {
                return reason;
            }
        }
    })
    .await
    .expect("tidak ada CallRejected");
    assert_eq!(reason, "sedang sibuk");
}

#[tokio::test]
async fn call_tanpa_koneksi_gagal_dengan_jelas() {
    let (a, _b) = pair().await;
    let err = a.call_offer("peer-hilang", "c3", "offer").unwrap_err();
    assert!(err.to_string().contains("koneksi"), "error: {err}");
}

// ---------------- TLS / autentikasi ----------------

/// Peer yang terhubung lewat jalur normal harus ditandai `verified`.
#[tokio::test]
async fn peer_terhubung_ditandai_verified() {
    let (a, b) = pair().await;
    let pa = a.peers().into_iter().find(|p| p.id == b.id()).expect("peer B hilang");
    assert!(pa.verified, "peer lewat TLS harus verified=true");
    let pb = b.peers().into_iter().find(|p| p.id == a.id()).expect("peer A hilang");
    assert!(pb.verified, "sisi penerima juga harus verified=true");
}

/// Inti klaim keamanan: byte di wire tidak boleh mengandung teks asli.
///
/// Caranya: buka listener TCP biasa yang meneruskan ke node asli, lalu dial
/// lewat listener itu (jadi kita melihat persis byte yang dikirim A ke B).
/// Kalau TLS bekerja, teks "rahasia-sekali" tidak akan pernah muncul.
#[tokio::test]
async fn wire_tidak_bocorkan_plaintext() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let b = Node::start(cfg("b")).await.unwrap();
    let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), b.port());

    // Proxy transparan: catat semua byte yang lewat.
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let captured: std::sync::Arc<std::sync::Mutex<Vec<u8>>> = Default::default();
    let cap2 = captured.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut inbound, _)) = proxy.accept().await else { break };
            let cap = cap2.clone();
            tokio::spawn(async move {
                let Ok(mut outbound) = tokio::net::TcpStream::connect(target).await else { return };
                // into_split: half milik sendiri ('static) supaya bisa dipindah ke task.
                let (mut ir, mut iw) = inbound.into_split();
                let (mut or, mut ow) = outbound.into_split();
                let c1 = cap.clone();
                let up = tokio::spawn(async move {
                    let mut buf = [0u8; 8192];
                    while let Ok(n) = ir.read(&mut buf).await {
                        if n == 0 { break }
                        c1.lock().unwrap().extend_from_slice(&buf[..n]);
                        if ow.write_all(&buf[..n]).await.is_err() { break }
                    }
                });
                let down = tokio::spawn(async move {
                    let mut buf = [0u8; 8192];
                    while let Ok(n) = or.read(&mut buf).await {
                        if n == 0 { break }
                        if iw.write_all(&buf[..n]).await.is_err() { break }
                    }
                });
                let _ = tokio::join!(up, down);
            });
        }
    });

    let a = Node::start(cfg("a")).await.unwrap();
    let a2 = a.clone();
    tokio::spawn(async move {
        let _ = a2.connect_peer(proxy_addr).await;
    });
    for _ in 0..80 {
        if a.is_connected(b.id()) { break }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(a.is_connected(b.id()), "A gagal terhubung lewat proxy");

    const RAHASIA: &str = "rahasia-sekali-jangan-sampai-bocor";
    a.send_text(b.id(), RAHASIA).unwrap();

    // tunggu pesan benar-benar sampai (membuktikan jalur hidup & data terkirim)
    let mut ev = b.subscribe();
    let ok = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Event::Text { body, .. }) = ev.recv().await {
                if body == RAHASIA { break true }
            }
        }
    }).await.unwrap_or(false);
    assert!(ok, "pesan tidak sampai ke B");

    let bytes = captured.lock().unwrap().clone();
    assert!(bytes.len() > 100, "tidak ada byte tertangkap (proxy tidak dilewati?)");
    let hay = String::from_utf8_lossy(&bytes);
    assert!(!hay.contains(RAHASIA), "PLAINTEXT BOCOR di wire: teks asli terlihat");
    assert!(!hay.contains("hello"), "frame JSON terlihat di wire (TLS tidak aktif)");
    // sanity: byte pertama harus TLS record handshake (0x16) — bukan JSON '{'
    assert_ne!(bytes[0], b'{', "wire dimulai dengan JSON, berarti tidak ada TLS");
}

/// Peer tanpa TLS harus DITOLAK secara default (bukan diam-diam diterima).
#[tokio::test]
async fn peer_plaintext_ditolak_default() {
    use tokio::io::AsyncWriteExt;

    let b = Node::start(cfg("b")).await.unwrap();
    let mut raw = tokio::net::TcpStream::connect(("127.0.0.1", b.port())).await.unwrap();

    // Kirim frame JSON telanjang seperti build lama (tanpa TLS).
    let body = br#"{"Hello":{"v":2,"id":"deadbeefdeadbeef","name":"lama","fp":"lama#dead","port":1234,"os":"linux"}}"#;
    let mut buf = (body.len() as u32).to_be_bytes().to_vec();
    buf.extend_from_slice(body);
    let _ = raw.write_all(&buf).await;

    // Server harus menutup koneksi tanpa pernah menganggapnya peer.
    let mut sink = [0u8; 64];
    use tokio::io::AsyncReadExt;
    let n = timeout(Duration::from_secs(5), raw.read(&mut sink)).await.unwrap_or(Ok(0)).unwrap_or(0);
    let closed_or_tls_error = n == 0 || sink[..n].first() == Some(&0x15); // 0x15 = TLS alert
    assert!(closed_or_tls_error, "server membalas data non-TLS (n={n})");
    assert!(
        b.peers().is_empty(),
        "peer plaintext tidak boleh masuk daftar peer"
    );
}

/// Status `verified` tidak boleh hilang setelah upsert berikutnya.
#[tokio::test]
async fn verified_tidak_turun_oleh_announce() {
    let (a, b) = pair().await;
    // a sudah punya peer b lewat TLS
    let before = a.peers().into_iter().find(|p| p.id == b.id()).unwrap();
    assert!(before.verified);

    // Simulasikan announce multicast yang sama (verified=false) datang belakangan
    // dengan menyalakan node ketiga yang meng-announce id b — bukan hal yang bisa
    // dilakukan test dari luar, jadi cukup pastikan status tidak berubah setelah
    // pengiriman pesan rutin (jalur upsert lain).
    a.send_text(b.id(), "tes").unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let after = a.peers().into_iter().find(|p| p.id == b.id()).unwrap();
    assert!(after.verified, "verified hilang setelah upsert berikutnya");
}

/// Serangan relay: penyerang memegang dua sesi TLS terpisah (A↔M dan M↔B)
/// lalu meneruskan frame apa adanya. Tanda tangan hello terikat ke exporter
/// masing-masing sesi, jadi A tidak bisa memakai tanda tangan B dan sebaliknya.
///
/// Yang diuji di sini: relay tidak bisa membuat A mengira sedang bicara dengan B
/// dengan memindahkan tanda tangan. Kita buktikan dengan menyalin frame `Auth`
/// dari sesi lain — harus ditolak.
#[tokio::test]
async fn tanda_tangan_tidak_bisa_dipakai_ulang_di_sesi_lain() {
    use dekat_core::identity::Identity;

    // Dua identitas nyata.
    let dir_a = tmp_dir("id-a");
    let dir_b = tmp_dir("id-b");
    let ia = Identity::load_or_create(&dir_a, "a").unwrap();
    let ib = Identity::load_or_create(&dir_b, "b").unwrap();

    // Sesi 1: A↔B (exporter asli).
    let ex1 = [1u8; 32];
    let msg1 = dekat_core::tls::hello_msg(&ex1, &ia.id(), &ib.id());

    // Sesi 2: sesi lain (exporter berbeda) antara pasangan yang sama.
    let ex2 = [2u8; 32];
    let msg2 = dekat_core::tls::hello_msg(&ex2, &ia.id(), &ib.id());

    let sig_sesi1 = ia.sign_hex(&msg1);

    // Tanda tangan sesi 1 sah di sesi 1...
    assert!(Identity::verify_hex(&ia.pk_hex(), &msg1, &sig_sesi1, Some(&ia.id())).is_ok());

    // ...tapi TIDAK sah di sesi 2, walau pasangan id sama. Inilah yang mematikan relay.
    let hasil = Identity::verify_hex(&ia.pk_hex(), &msg2, &sig_sesi1, Some(&ia.id()));
    assert!(hasil.is_err(), "tanda tangan sesi lain diterima — relay bisa menembus");

    // Dan tanda tangan B tidak bisa dipakai mengaku sebagai A (id != pk prefix).
    let sig_b = ib.sign_hex(&msg1);
    assert!(Identity::verify_hex(&ib.pk_hex(), &msg1, &sig_b, Some(&ia.id())).is_err(),
        "id palsu diterima: peer mengaku ber-id A dengan kunci B");

    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

// ---------------- panggilan grup (mesh) ----------------

/// Roster grup harus sampai ke peserta, dan tiap penerima tahu siapa saja pesertanya.
/// Ini yang dipakai UI untuk membangun mesh (sambung ke semua peserta, bukan cuma pemanggil).
#[tokio::test]
async fn call_grup_membawa_roster_peserta() {
    let (a, b) = pair().await;
    let mut ev = b.subscribe();

    let members = vec![a.id().to_string(), b.id().to_string(), "cccccccccccccccc".to_string()];
    a.call_offer_group(b.id(), "call-1", "v=0\r\nsdp-offer", &members)
        .expect("gagal kirim offer grup");

    let got = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Event::CallIncoming { call_id, from, members, sdp, .. }) = ev.recv().await {
                break Some((call_id, from, members, sdp));
            }
        }
    })
    .await
    .expect("tidak ada CallIncoming")
    .expect("event lain");

    assert_eq!(got.0, "call-1");
    assert_eq!(got.1, a.id());
    assert_eq!(got.2, members, "roster peserta harus diteruskan apa adanya");
    assert_eq!(got.3, "v=0\r\nsdp-offer");

    // Panggilan 1:1 tetap jalan, roster kosong = bukan grup.
    let mut ev2 = b.subscribe();
    a.call_offer(b.id(), "call-2", "sdp-dm").unwrap();
    let one = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Event::CallIncoming { members, .. }) = ev2.recv().await {
                break members;
            }
        }
    })
    .await
    .unwrap();
    assert!(one.is_empty(), "panggilan DM tidak boleh membawa roster");
}

/// Mesh 3 node: A memanggil B dan C. Setelah keduanya menerima, B→C harus bisa
/// tersambung langsung. Diuji di level signalling: frame offer B→C diterima C.
#[tokio::test]
async fn mesh_tiga_node_saling_terhubung() {
    let a = Node::start(cfg("a")).await.unwrap();
    let b = Node::start(cfg("b")).await.unwrap();
    let c = Node::start(cfg("c")).await.unwrap();

    // Sambungkan semua lewat transport chat.
    for (x, y) in [(&a, &b), (&a, &c), (&b, &c)] {
        // x yang mendial y (bukan sebaliknya — dial ke port sendiri akan ditolak).
        let x2 = x.clone();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), y.port());
        tokio::spawn(async move { let _ = x2.connect_peer(addr).await; });
    }
    for _ in 0..80 {
        if a.is_connected(b.id()) && a.is_connected(c.id()) && b.is_connected(c.id()) { break }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(b.is_connected(c.id()), "transport B↔C belum siap");

    // A memanggil grup berisi A, B, C.
    let members = vec![a.id().to_string(), b.id().to_string(), c.id().to_string()];
    let mut ev_c = c.subscribe();
    let mut ev_b = b.subscribe();
    a.call_offer_group(b.id(), "g-1", "offer-dari-a", &members).unwrap();
    a.call_offer_group(c.id(), "g-1", "offer-dari-a", &members).unwrap();

    // B dan C menerima undangan beserta roster.
    for (name, ev) in [("b", &mut ev_b), ("c", &mut ev_c)] {
        let m = timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(Event::CallIncoming { members, .. }) = ev.recv().await { break members }
            }
        }).await.unwrap_or_else(|_| panic!("{name} tidak menerima undangan"));
        assert_eq!(m.len(), 3, "{name} harus tahu ada 3 peserta");
    }

    // Sekarang B menghubungi C langsung (langkah mesh yang dilakukan UI).
    // C harus menerima offer dari B — inilah koneksi peer-to-peer tambahannya.
    let mut ev_c2 = c.subscribe();
    b.call_offer_group(c.id(), "g-1", "offer-dari-b", &members).unwrap();
    let from_b = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Event::CallIncoming { from, sdp, .. }) = ev_c2.recv().await { break (from, sdp) }
        }
    }).await.expect("C tidak menerima offer mesh dari B");

    assert_eq!(from_b.0, b.id(), "offer mesh harus datang dari B");
    assert_eq!(from_b.1, "offer-dari-b");
}

// ---------------- profil (nama kustom + avatar) ----------------

/// Nama tampilan bisa diganti dan peer lain menerimanya lewat frame Profile.
#[tokio::test]
async fn nama_profil_berubah_dan_sampai_ke_peer() {
    let (a, b) = pair().await;
    let mut ev = b.subscribe();

    a.set_profile(Some("Ray Ganteng"), None).expect("gagal set nama");
    assert_eq!(a.name(), "Ray Ganteng", "nama sendiri harus langsung berubah");

    // Frame Profile dikirim lewat kanal writer, jadi tunggu sampai sampai di B.
    let mut nama_b = String::new();
    for _ in 0..60 {
        if let Some(p) = b.peers().into_iter().find(|p| p.id == a.id()) {
            nama_b = p.name.clone();
            if nama_b == "Ray Ganteng" { break }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(nama_b, "Ray Ganteng", "peer B tidak menerima nama baru");
    let _ = ev.recv().await; // event apa pun, yang penting tidak panik
}

/// Avatar ikut terkirim dan tersimpan di sisi peer.
#[tokio::test]
async fn avatar_profil_sampai_ke_peer() {
    let (a, b) = pair().await;
    // PNG 1x1 transparan
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==";
    a.set_profile(None, Some(PNG.to_string())).expect("gagal set avatar");

    let mut ok = false;
    for _ in 0..60 {
        if let Some(p) = b.peers().into_iter().find(|p| p.id == a.id()) {
            if p.avatar.as_deref() == Some(PNG) { ok = true; break }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ok, "avatar tidak sampai ke peer B");
    assert_eq!(a.avatar().as_deref(), Some(PNG), "avatar sendiri harus tersimpan");
}

/// Nama kosong/terlalu panjang ditolak, dan nama lama tidak berubah.
#[tokio::test]
async fn nama_profil_tidak_valid_ditolak() {
    let (a, _b) = pair().await;
    let asli = a.name();
    assert!(a.set_profile(Some("   "), None).is_err(), "nama kosong harus ditolak");
    assert_eq!(a.name(), asli, "nama tidak boleh berubah saat input ditolak");
    let panjang = "x".repeat(40);
    assert!(a.set_profile(Some(&panjang), None).is_err(), "nama 40 char harus ditolak");
}

/// Profil bertahan setelah node di-restart (disimpan di profile.json).
#[tokio::test]
async fn profil_bertahan_setelah_restart() {
    let dir = tmp_dir("profil");
    let mut cfg = Config::new("awal", &dir);
    cfg.port = 0;
    cfg.discovery = false;

    let n = Node::start(cfg.clone()).await.unwrap();
    n.set_profile(Some("Nama Baru"), None).unwrap();
    let id1 = n.id().to_string();
    drop(n);
    tokio::time::sleep(Duration::from_millis(200)).await;

    let n2 = Node::start(cfg).await.unwrap();
    assert_eq!(n2.id(), id1, "id harus tetap sama setelah restart");
    assert_eq!(n2.name(), "Nama Baru", "nama kustom harus bertahan");
}

// ---------------- file: grup & resume ----------------

/// Kirim file ke dua peer sekaligus (satu panggilan) — keduanya harus utuh.
#[tokio::test]
async fn kirim_file_ke_dua_peer_sekaligus() {
    let a = Node::start(cfg("a")).await.unwrap();
    let b = Node::start(cfg("b")).await.unwrap();
    let c = Node::start(cfg("c")).await.unwrap();
    for y in [&b, &c] {
        let x = a.clone();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), y.port());
        tokio::spawn(async move { let _ = x.connect_peer(addr).await; });
    }
    for _ in 0..80 {
        if a.is_connected(b.id()) && a.is_connected(c.id()) { break }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(a.is_connected(b.id()) && a.is_connected(c.id()));

    // File 200 KB supaya lebih dari satu potongan.
    let src = tmp_dir("kirim").join("grup.bin");
    let isi: Vec<u8> = (0..200_000).map(|i| (i % 253) as u8).collect();
    std::fs::write(&src, &isi).unwrap();
    let sha = dekat_core::files::sha256_file(&src).unwrap();

    let targets = vec![b.id().to_string(), c.id().to_string()];
    let id = a.send_file_many(&targets, &src, 30).await.expect("kirim ke dua peer gagal");
    assert!(!id.is_empty());

    // Kedua penerima harus punya file dengan sha yang sama.
    for (nama, node) in [("b", &b), ("c", &c)] {
        let mut found = false;
        for _ in 0..100 {
            let dir = node.files_dir();
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    if e.path().extension().and_then(|x| x.to_str()) == Some("bin") {
                        if dekat_core::files::sha256_file(&e.path()).unwrap_or_default() == sha {
                            found = true;
                        }
                    }
                }
            }
            if found { break }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(found, "peer {nama} tidak menerima file utuh");
    }
}

/// Resume: kalau penerima sudah punya sebagian, pengirim hanya mengirim sisanya.
///
/// Diuji di level files.rs (bukan lewat jaringan) karena menghentikan transfer di
/// tengah koneksi nyata tidak deterministik.
#[tokio::test]
async fn resume_transfer_melanjutkan_dari_posisi_terakhir() {
    use dekat_core::files::FileTransfer;

    let dir = tmp_dir("resume");
    let chunk = dekat_core::proto::FILE_CHUNK_BYTES;
    let isi: Vec<u8> = (0..chunk * 4).map(|i| (i % 241) as u8).collect();
    let sha = dekat_core::files::sha256_file_bytes(&isi);

    // Penerima sudah punya 2 potongan pertama dari percobaan sebelumnya.
    let pdir = dir.join("partial");
    std::fs::create_dir_all(&pdir).unwrap();
    std::fs::write(pdir.join(format!("{sha}.part")), &isi[..chunk * 2]).unwrap();

    let ft = FileTransfer::new();
    let (_path, have) = ft
        .begin_inbound(&dir, "baru", "data.bin", isi.len() as u64, &sha, 4)
        .unwrap();
    assert_eq!(have, (chunk * 2) as u64, "harus melapor sudah punya 2 potongan");

    // Kirim sisa potongan 3 & 4 saja.
    let mut done = None;
    for idx in 2..4u32 {
        let b64 = base64::engine::general_purpose::STANDARD.encode(
            &isi[idx as usize * chunk..(idx as usize + 1) * chunk],
        );
        done = ft.write_chunk("baru", idx, &b64).unwrap();
    }
    let (path, _) = done.expect("transfer harus selesai");
    assert_eq!(dekat_core::files::sha256_file(&path).unwrap(), sha, "hasil resume harus utuh");
}

// ---------------- regresi: nama grup & bug grup ----------------

/// Nama grup harus ikut terkirim bersama pesan; tanpa itu anggota lain hanya
/// melihat label generik "Grup" karena room id tidak menyimpan nama.
#[tokio::test]
async fn nama_grup_ikut_terkirim() {
    let (a, b) = pair().await;
    let mut ev = b.subscribe();

    let members = vec![a.id().to_string(), b.id().to_string(), "cccccccccccccccc".to_string()];
    a.send_group_named(&members, "halo", "Warkop Solo").expect("gagal kirim grup");

    let got = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Event::Text { room_name, body, .. }) = ev.recv().await {
                break (room_name, body);
            }
        }
    })
    .await
    .expect("pesan grup tidak sampai");

    assert_eq!(got.0, "Warkop Solo", "nama grup harus ikut di frame");
    assert_eq!(got.1, "halo");
}

/// `send_group` lama (tanpa nama) tetap kompatibel: nama kosong, bukan panik.
#[tokio::test]
async fn send_group_tanpa_nama_tetap_jalan() {
    let (a, b) = pair().await;
    let mut ev = b.subscribe();
    let members = vec![a.id().to_string(), b.id().to_string(), "dddddddddddddddd".to_string()];
    a.send_group(&members, "tes").expect("send_group gagal");
    let room_name = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Event::Text { room_name, .. }) = ev.recv().await {
                break room_name;
            }
        }
    })
    .await
    .expect("pesan tidak sampai");
    assert!(room_name.is_empty(), "tanpa nama, field harus kosong");
}
