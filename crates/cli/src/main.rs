//! lanchat CLI — harness M0 buat nguji core tanpa UI.
//!
//! Pemakaian:
//!   lanchat --name ray --port 41235 --seed 10.10.1.28:41236
//!
//! Perintah di dalam sesi:
//!   <teks>            kirim ke semua peer (broadcast)
//!   /dm <id|nama> <teks>
//!   /peers            daftar peer
//!   /ping             info node
//!   /quit

use anyhow::Result;
use lanchat_core::{proto::fmt_hms, Config, Event, Node, BROADCAST};
use std::{
    net::SocketAddr,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::io::{AsyncBufReadExt, BufReader};

fn parse_args() -> Result<Config> {
    let mut name = std::env::var("USER").unwrap_or_else(|_| "user".into());
    let mut port: u16 = lanchat_core::proto::DEFAULT_APP_PORT;
    let mut data_dir = dirs_or_home().join(".lanchat");
    let mut seeds: Vec<SocketAddr> = Vec::new();

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--name" | "-n" => name = it.next().unwrap_or_default(),
            "--port" | "-p" => port = it.next().unwrap_or_default().parse()?,
            "--dir" => data_dir = PathBuf::from(it.next().unwrap_or_default()),
            "--seed" | "--peer" => {
                let v = it.next().unwrap_or_default();
                seeds.push(v.parse()?);
            }
            "--help" | "-h" => {
                println!(
                    "lanchat M0\n  --name <nama>\n  --port <port tcp>\n  --dir <data dir>\n  --seed <ip:port> (boleh berkali-kali)"
                );
                std::process::exit(0);
            }
            other => anyhow::bail!("argumen tidak dikenal: {other}"),
        }
    }

    Ok(Config {
        name,
        port,
        data_dir,
        seeds,
        discovery: true,
    })
}

fn dirs_or_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn now_hms() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    fmt_hms(ms)
}

fn print_event(e: Event) {
    match e {
        Event::PeerUp(p) => println!(
            "  + {} [{}] {}  {}:{}",
            p.name,
            p.os,
            &p.id[..6],
            p.addr.ip(),
            p.addr.port()
        ),
        Event::PeerDown(id) => println!("  - {} keluar", &id[..6.min(id.len())]),
        Event::Text {
            from,
            from_name,
            to,
            body,
            ts,
            mine,
            ..
        } => {
            if mine {
                let dest = if to == BROADCAST {
                    "semua".to_string()
                } else {
                    to[..6.min(to.len())].to_string()
                };
                println!("[{}] aku → {}: {}", fmt_hms(ts), dest, body);
            } else {
                let _ = from;
                println!("[{}] {}: {}", fmt_hms(ts), from_name, body);
            }
        }
        Event::Ack { id, ts } => println!("[{}] ✓ terkirim ({})", fmt_hms(ts), &id[..8]),
        Event::Typing { from, to, on } => {
            if on {
                println!("  … {} sedang mengetik ({})", &from[..6.min(from.len())], to)
            }
        }
        Event::Read { from, up_to_ts, .. } => println!(
            "[{}] {} membaca pesanmu (sampai ts {})",
            fmt_hms(up_to_ts),
            &from[..6.min(from.len())],
            up_to_ts
        ),
        Event::PeerStatus { id, status } => println!(
            "  ~ {} sekarang {}",
            &id[..6.min(id.len())],
            status.as_str()
        ),
        Event::FileOffer {
            name, size, mine: false, ..
        } => println!("  ↓ menerima file: {} ({} byte)", name, size),
        Event::FileProgress {
            name,
            sent_bytes,
            total_bytes,
            outgoing,
            ..
        } => {
            let pct = if total_bytes > 0 { sent_bytes * 100 / total_bytes } else { 0 };
            println!(
                "  {} {}: {}%",
                if outgoing { "↑" } else { "↓" },
                name,
                pct
            );
        }
        Event::FileReceived { name, path, .. } => {
            println!("  ✓ file diterima: {} → {}", name, path)
        }
        Event::FileFailed { name, reason, .. } => println!("  ✗ transfer gagal {}: {}", name, reason),
        Event::CallIncoming {
            call_id,
            from_name,
            ..
        } => println!("  ☎ PANGGILAN MASUK dari {from_name} (id {call_id}) — jawab dengan /answer <nama> {call_id} accept"),
        Event::CallAccepted { call_id, .. } => println!("  ☎ panggilan {call_id} diterima"),
        Event::CallRejected { call_id, reason, .. } => {
            println!("  ☎ panggilan {call_id} ditolak: {reason}")
        }
        Event::CallIce { .. } => println!("  ☎ ICE diterima"),
        Event::CallEnded { call_id, reason, .. } => {
            println!("  ☎ panggilan {call_id} berakhir: {reason}")
        }
        Event::FileOffer { .. } => {}
        Event::Warn(w) => println!("! {w}"),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn".into()),
        )
        .with_target(false)
        .init();

    let cfg = parse_args()?;
    let node = Node::start(cfg).await?;

    println!("lanchat M0 — {} [{}]", node.name(), node.id());
    println!("listening TCP :{}", node.port());
    println!("perintah: <teks> = broadcast, /dm <id> <teks>, /peers, /quit\n");

    // call_id terakhir yang masuk, biar `/answer <peer> auto accept` bisa dipakai
    let last_call: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let last_call_ev = last_call.clone();

    let mut ev = node.subscribe();
    tokio::spawn(async move {
        loop {
            match ev.recv().await {
                Ok(e) => {
                    if let Event::CallIncoming { call_id, .. } = &e {
                        *last_call_ev.lock().unwrap() = Some(call_id.clone());
                    }
                    print_event(e)
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    println!("! {n} event terlewat")
                }
                Err(_) => break,
            }
        }
    });

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "/quit" || line == "/exit" {
            break;
        }
        if line == "/peers" {
            let peers = node.peers();
            if peers.is_empty() {
                println!("  (belum ada peer)");
            }
            for p in peers {
                println!(
                    "  {:<12} {:<8} {:<6} {}  {}",
                    p.name,
                    p.os,
                    &p.id[..6],
                    p.addr,
                    if node.is_connected(&p.id) {
                        "connected"
                    } else {
                        "-"
                    }
                );
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("/file ") {
            let mut parts = rest.splitn(2, ' ');
            let who = parts.next().unwrap_or_default();
            let path = parts.next().unwrap_or_default().trim();
            if who.is_empty() || path.is_empty() {
                println!("  pakai: /file <id|nama> <path>");
                continue;
            }
            match node.find_peer(who) {
                Some(p) => {
                    let node2 = node.clone();
                    let p2 = p.clone();
                    let path2 = std::path::PathBuf::from(path);
                    println!("  mengirim {} ke {}…", path, p.name);
                    tokio::spawn(async move {
                        match node2.send_file(&p2.id, &path2, 30).await {
                            Ok(id) => println!("  ✓ file terkirim ({})", &id[..8]),
                            Err(e) => println!("  ✗ gagal: {e}"),
                        }
                    });
                }
                None => println!("  peer '{who}' tidak ketemu"),
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("/call ") {
            let who = rest.trim();
            match node.find_peer(who) {
                Some(p) => {
                    let call_id = format!("c-{}", &lanchat_core::proto::new_msg_id()[..8]);
                    // simpan juga di sisi pemanggil, supaya /hangup auto bisa dipakai
                    *last_call.lock().unwrap() = Some(call_id.clone());
                    match node.call_offer(&p.id, &call_id, "dummy-offer-sdp") {
                        Ok(()) => println!("  ☎ memanggil {} ({})", p.name, call_id),
                        Err(e) => println!("  gagal: {e}"),
                    }
                }
                None => println!("  peer '{who}' tidak ketemu"),
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("/answer ") {
            let mut it = rest.splitn(3, ' ');
            let who = it.next().unwrap_or_default();
            let call_id = it.next().unwrap_or_default();
            let verdict = it.next().unwrap_or("accept").trim();
            let call_id = if call_id == "auto" {
                last_call.lock().unwrap().clone().unwrap_or_default()
            } else {
                call_id.to_string()
            };
            let call_id = call_id.as_str();
            if call_id.is_empty() {
                println!("  belum ada panggilan masuk");
                continue;
            }
            if let Some(p) = node.find_peer(who) {
                let accept = verdict != "reject";
                match node.call_answer(&p.id, call_id, accept, "dummy-answer-sdp", verdict) {
                    Ok(()) => println!("  ☎ jawab {} : {}", call_id, verdict),
                    Err(e) => println!("  gagal: {e}"),
                }
            } else {
                println!("  peer '{who}' tidak ketemu");
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("/hangup ") {
            let mut it = rest.splitn(2, ' ');
            let who = it.next().unwrap_or_default();
            let call_id = it.next().unwrap_or_default();
            let call_id = if call_id == "auto" {
                last_call.lock().unwrap().clone().unwrap_or_default()
            } else {
                call_id.to_string()
            };
            let call_id = call_id.as_str();
            if call_id.is_empty() {
                println!("  belum ada panggilan");
                continue;
            }
            if let Some(p) = node.find_peer(who) {
                let _ = node.call_end(&p.id, call_id, "ditutup dari CLI");
                println!("  ☎ tutup {call_id}");
            }
            continue;
        }
        if line == "/files" {
            let dir = node.files_dir();
            println!("  folder: {}", dir.display());
            continue;
        }
        if line == "/ping" {
            println!(
                "  {} [{}] tcp:{} peers:{}",
                node.name(),
                node.id(),
                node.port(),
                node.peers().len()
            );
            continue;
        }
        if let Some(rest) = line.strip_prefix("/dm ") {
            let mut parts = rest.splitn(2, ' ');
            let who = parts.next().unwrap_or_default();
            let body = parts.next().unwrap_or_default().trim();
            if who.is_empty() || body.is_empty() {
                println!("  pakai: /dm <id|nama> <teks>");
                continue;
            }
            match node.find_peer(who) {
                Some(p) => {
                    if let Err(e) = node.send_text(&p.id, body) {
                        println!("  gagal: {e}");
                    }
                }
                None => println!("  peer '{who}' tidak ketemu"),
            }
            continue;
        }
        if let Err(e) = node.send_text(BROADCAST, line) {
            println!("  gagal: {e}");
        }
    }

    println!("\n{} keluar. {}", now_hms(), "bye 👋");
    Ok(())
}