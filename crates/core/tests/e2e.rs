//! Integration test: dua node nyata, socket nyata, satu proses.
//!
//! Yang diuji: handshake, pengiriman DM + ack, pesan grup (fan-out), typing.
//! Tidak butuh jaringan luar — semua lewat loopback.

use lanchat_core::{Config, Event, Node};
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
    let p = std::env::temp_dir().join(format!("lanchat-it-{}-{}-{}", tag, std::process::id(), n));
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
    assert_eq!(room, lanchat_core::room_id(&members));

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
    let dir_a = std::env::temp_dir().join(format!("lanchat-sendfile-{}", std::process::id()));
    std::fs::create_dir_all(&dir_a).unwrap();
    let src = dir_a.join("dokumen.bin");
    {
        let mut f = std::fs::File::create(&src).unwrap();
        let data: Vec<u8> = (0..300 * 1024).map(|i| (i % 253) as u8).collect();
        f.write_all(&data).unwrap();
    }
    let expected = lanchat_core::files::sha256_file(&src).unwrap();

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
    assert_eq!(lanchat_core::files::sha256_file(std::path::Path::new(&path)).unwrap(), expected);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 300 * 1024);

    let msg_id = timeout(Duration::from_secs(20), send).await.unwrap().unwrap();
    assert!(msg_id.is_ok(), "pengirim melaporkan gagal: {msg_id:?}");
    let _ = std::fs::remove_dir_all(&dir_a);
}

#[tokio::test]
async fn kirim_file_tanpa_koneksi_gagal_dengan_jelas() {
    let (a, _b) = pair().await;
    let src = std::env::temp_dir().join(format!("lanchat-nofile-{}.bin", std::process::id()));
    std::fs::write(&src, b"isi").unwrap();
    let err = a.send_file("peer-yang-tidak-ada", &src, 1).await.unwrap_err();
    assert!(err.to_string().contains("koneksi"), "pesan error: {err}");
    let _ = std::fs::remove_file(&src);
}
