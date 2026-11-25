//! Presence / discovery antar node di LAN yang sama.
//!
//! Strategi berlapis, karena banyak jaringan kampus/kantor memblok salah satunya:
//! 1. UDP multicast ke `239.255.42.99:41234` (utama)
//! 2. limited broadcast ke `255.255.255.255:41234` (fallback)
//! 3. seed manual `ip:port` dari argumen CLI (terakhir, buat jaringan aneh)
//!
//! Socket dibuka dengan SO_REUSEADDR + SO_REUSEPORT supaya beberapa instance
//! di satu mesin tetap bisa menerima announce yang sama.

use crate::proto::{now_ms, Announce, ANNOUNCE_INTERVAL_SECS, DISCOVERY_PORT, MULTICAST_ADDR};
use anyhow::{Context, Result};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket as StdUdpSocket},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::UdpSocket, sync::mpsc};

/// Announce yang berhasil diterima + dari IP mana asalnya.
#[derive(Debug, Clone)]
pub struct Seen {
    pub announce: Announce,
    pub from: IpAddr,
}

/// IP lokal yang dipakai untuk keluar (trik connect UDP, tidak mengirim paket).
pub fn local_ip() -> IpAddr {
    StdUdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("8.8.8.8:80")?;
            s.local_addr()
        })
        .map(|a| a.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

/// Buka socket discovery dan jalankan loop announce + receive di background.
pub fn spawn(
    announce: Arc<Mutex<Announce>>,
    seeds: Vec<SocketAddr>,
    tx: mpsc::Sender<Seen>,
) -> Result<()> {
    use socket2::{Domain, Protocol, Socket, Type};

    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .context("buat socket UDP")?;
    sock.set_reuse_address(true)?;
    #[cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos")))]
    {
        let _ = sock.set_reuse_port(true);
    }

    let bind_addr: SocketAddr = (Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT).into();
    sock.bind(&bind_addr.into())
        .with_context(|| format!("bind UDP {bind_addr} (port dipakai proses lain?)"))?;

    let group: Ipv4Addr = MULTICAST_ADDR.parse()?;
    if let Err(e) = sock.join_multicast_v4(&group, &Ipv4Addr::UNSPECIFIED) {
        tracing::warn!("join multicast gagal (lanjut ke broadcast): {e}");
    }
    // join juga di interface utama, jaga-jaga kalau INADDR_ANY tidak cukup
    if let IpAddr::V4(ip) = local_ip() {
        let _ = sock.join_multicast_v4(&group, &ip);
    }

    let _ = sock.set_multicast_loop_v4(true);
    sock.set_nonblocking(true)?;
    let std_sock: StdUdpSocket = sock.into();
    let sock = Arc::new(UdpSocket::from_std(std_sock)?);

    let me_id = announce.lock().unwrap().id.clone();

    // --- pengirim presence ---
    let s = sock.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(ANNOUNCE_INTERVAL_SECS));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            // status/nama bisa berubah kapan saja; baca ulang tiap tick
            let mut me0 = announce.lock().unwrap().clone();
            me0.ts = now_ms();
            let bytes = match serde_json::to_vec(&me0) {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!("serialize announce: {e}");
                    continue;
                }
            };
            let _ = s.send_to(&bytes, (group, DISCOVERY_PORT)).await;
            let _ = s
                .send_to(&bytes, (Ipv4Addr::BROADCAST, DISCOVERY_PORT))
                .await;
            for seed in &seeds {
                let _ = s.send_to(&bytes, seed).await;
            }
        }
    });

    // --- penerima presence ---
    tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            match sock.recv_from(&mut buf).await {
                Ok((n, from)) => {
                    let parsed: Result<Announce, _> = serde_json::from_slice(&buf[..n]);
                    match parsed {
                        Ok(a) if a.v == crate::proto::PROTO_VERSION && a.id != me_id => {
                            if tx.send(Seen { announce: a, from: from.ip() }).await.is_err() {
                                return; // node sudah berhenti
                            }
                        }
                        Ok(_) => {}
                        Err(_) => tracing::debug!("paket discovery tidak dikenal dari {from}"),
                    }
                }
                Err(e) => {
                    tracing::debug!("recv discovery: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    });

    Ok(())
}
