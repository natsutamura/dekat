#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! App desktop LAN Chat (Tauri v2). Frontend statis ada di `ui/`.

use lanchat_core::{
    is_room, room_id, Attachment, Config, Event, Node, Status, Store, StoredMessage, Thread,
    BROADCAST,
};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager, State};

struct App {
    node: Node,
    store: Arc<Store>,
    /// thread yang sedang dibuka di UI (buat nentuin unread)
    active: Mutex<Option<String>>,
}

type AppState = Arc<App>;

fn e2s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[derive(Serialize, Clone)]
struct Me {
    id: String,
    name: String,
    fp: String,
    port: u16,
    status: String,
}

#[derive(Serialize, Clone)]
struct PeerDto {
    id: String,
    name: String,
    os: String,
    fp: String,
    addr: String,
    status: String,
    online: bool,
}

#[derive(Serialize, Clone, Default)]
struct Settings {
    theme: String,
    accent: String,
    bubble_in: String,
    bubble_out: String,
    radius: f64,
    scale: f64,
}

#[derive(Serialize)]
struct Bootstrap {
    me: Me,
    threads: Vec<Thread>,
    peers: Vec<PeerDto>,
    settings: Settings,
}

fn read_settings(store: &Store) -> Settings {
    let get = |k: &str| store.get_setting(k).ok().flatten();
    Settings {
        theme: get("theme").unwrap_or_else(|| "light".into()),
        accent: get("accent").unwrap_or_else(|| "#0b84ff".into()),
        bubble_in: get("bubble_in").unwrap_or_else(|| "".into()),
        bubble_out: get("bubble_out").unwrap_or_else(|| "".into()),
        radius: get("radius")
            .and_then(|v| v.parse().ok())
            .unwrap_or(18.0),
        scale: get("scale").and_then(|v| v.parse().ok()).unwrap_or(1.0),
    }
}

fn peers_of(node: &Node) -> Vec<PeerDto> {
    node.peers()
        .into_iter()
        .map(|p| PeerDto {
            id: p.id,
            name: p.name,
            os: p.os,
            fp: p.fp,
            addr: p.addr.to_string(),
            status: p.status.as_str().to_string(),
            online: p.online,
        })
        .collect()
}

fn me_of(node: &Node) -> Me {
    Me {
        id: node.id().to_string(),
        name: node.name().to_string(),
        fp: node.fp().to_string(),
        port: node.port(),
        status: node.status().as_str().to_string(),
    }
}

#[tauri::command]
#[allow(non_snake_case)] // argumen camelCase biar cocok dengan invoke() di JS
fn bootstrap(state: State<AppState>) -> Result<Bootstrap, String> {
    let st = state.inner().clone();
    Ok(Bootstrap {
        me: me_of(&st.node),
        threads: st.store.threads().map_err(e2s)?,
        peers: peers_of(&st.node),
        settings: read_settings(&st.store),
    })
}

#[tauri::command]
#[allow(non_snake_case)] // argumen camelCase biar cocok dengan invoke() di JS
fn messages(state: State<AppState>, threadId: String, limit: Option<usize>) -> Result<Vec<StoredMessage>, String> {
    state
        .store
        .messages(&threadId, limit.unwrap_or(500))
        .map_err(e2s)
}

#[tauri::command]
#[allow(non_snake_case)] // argumen camelCase biar cocok dengan invoke() di JS
fn open_dm(state: State<AppState>, peerId: String) -> Result<Thread, String> {
    let st = state.inner().clone();
    if let Some(t) = st.store.thread(&peerId).map_err(e2s)? {
        return Ok(t);
    }
    let name = st
        .node
        .peer(&peerId)
        .map(|p| p.name)
        .unwrap_or_else(|| peerId.clone());
    let t = Thread {
        id: peerId.clone(),
        kind: "dm".into(),
        name,
        members: vec![peerId],
        last_ts: 0,
        unread: 0,
        manual_unread: false,
        preview: String::new(),
    };
    st.store.ensure_thread(&t).map_err(e2s)?;
    Ok(t)
}

#[tauri::command]
#[allow(non_snake_case)] // argumen camelCase biar cocok dengan invoke() di JS
fn create_group(
    state: State<AppState>,
    name: String,
    members: Vec<String>,
) -> Result<Thread, String> {
    let st = state.inner().clone();
    let mut all = members;
    all.push(st.node.id().to_string());
    all.sort();
    all.dedup();
    if all.len() < 3 {
        return Err("grup butuh minimal 2 peer selain kamu".into());
    }
    let id = room_id(&all);
    let t = Thread {
        id,
        kind: "group".into(),
        name: if name.trim().is_empty() {
            "Grup baru".into()
        } else {
            name.trim().to_string()
        },
        members: all,
        last_ts: 0,
        unread: 0,
        manual_unread: false,
        preview: String::new(),
    };
    st.store.ensure_thread(&t).map_err(e2s)?;
    Ok(t)
}

#[tauri::command]
#[allow(non_snake_case)] // argumen camelCase biar cocok dengan invoke() di JS
fn send(state: State<AppState>, threadId: String, body: String) -> Result<(), String> {
    let st = state.inner().clone();
    if body.trim().is_empty() {
        return Ok(());
    }
    let t = st
        .store
        .thread(&threadId)
        .map_err(e2s)?
        .ok_or_else(|| "thread tidak dikenal".to_string())?;
    match t.kind.as_str() {
        "group" => {
            st.node.send_group(&t.members, &body).map_err(e2s)?;
        }
        "broadcast" => {
            st.node.send_text(BROADCAST, &body).map_err(e2s)?;
        }
        _ => {
            st.node.send_text(&t.id, &body).map_err(e2s)?;
        }
    }
    Ok(())
}

#[tauri::command]
#[allow(non_snake_case)] // argumen camelCase biar cocok dengan invoke() di JS
fn typing(state: State<AppState>, threadId: String, on: bool) -> Result<(), String> {
    let st = state.inner().clone();
    let t = st.store.thread(&threadId).map_err(e2s)?;
    if let Some(t) = t {
        if t.kind == "dm" {
            let _ = st.node.send_typing(&t.id, on);
        }
    }
    Ok(())
}

#[tauri::command]
#[allow(non_snake_case)] // argumen camelCase biar cocok dengan invoke() di JS
fn set_active(state: State<AppState>, threadId: Option<String>) -> Result<(), String> {
    let st = state.inner().clone();
    *st.active.lock().unwrap() = threadId.clone();
    if let Some(id) = threadId {
        st.store.mark_read(&id).map_err(e2s)?;
    }
    Ok(())
}

#[tauri::command]
#[allow(non_snake_case)] // argumen camelCase biar cocok dengan invoke() di JS
fn mark_read(state: State<AppState>, threadId: String) -> Result<(), String> {
    let st = state.inner().clone();
    st.store.mark_read(&threadId).map_err(e2s)?;
    // kalau ini DM, kabari peer bahwa pesannya sudah dibaca
    if !is_room(&threadId) && threadId != BROADCAST {
        let last = st
            .store
            .messages(&threadId, 1)
            .map_err(e2s)?
            .last()
            .map(|m| m.ts)
            .unwrap_or(0);
        if last > 0 {
            let _ = st.node.send_read(&threadId, last);
        }
    }
    Ok(())
}

#[tauri::command]
#[allow(non_snake_case)] // argumen camelCase biar cocok dengan invoke() di JS
fn set_unread(state: State<AppState>, threadId: String, unread: bool) -> Result<(), String> {
    let st = state.inner().clone();
    st.store
        .set_unread(&threadId, if unread { 1 } else { 0 }, unread)
        .map_err(e2s)
}

#[tauri::command]
#[allow(non_snake_case)] // argumen camelCase biar cocok dengan invoke() di JS
fn clear_chat(state: State<AppState>, threadId: String) -> Result<(), String> {
    let st = state.inner().clone();
    st.store.clear_thread(&threadId).map_err(e2s)
}

#[tauri::command]
#[allow(non_snake_case)] // argumen camelCase biar cocok dengan invoke() di JS
fn set_status(state: State<AppState>, status: String) -> Result<(), String> {
    let st = state.inner().clone();
    st.node.set_status(Status::parse(&status));
    Ok(())
}

#[tauri::command]
#[allow(non_snake_case)] // argumen camelCase biar cocok dengan invoke() di JS
fn set_setting(state: State<AppState>, key: String, value: String) -> Result<(), String> {
    state.store.set_setting(&key, &value).map_err(e2s)
}

/// Pilih file lewat dialog OS, lalu kirim ke thread (async supaya UI tidak beku).
#[tauri::command]
#[allow(non_snake_case)]
async fn send_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    threadId: String,
) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;

    let picked = app
        .dialog()
        .file()
        .set_title("Pilih file untuk dikirim")
        .blocking_pick_file();
    let path = match picked {
        Some(f) => f.into_path().map_err(e2s)?,
        None => return Ok("dibatalkan".into()),
    };

    let st = state.inner().clone();
    let t = st
        .store
        .thread(&threadId)
        .map_err(e2s)?
        .ok_or_else(|| "thread tidak dikenal".to_string())?;
    if t.kind != "dm" {
        return Err("kirim file untuk sementara hanya di chat pribadi (DM)".into());
    }

    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

    match st.node.send_file(&t.id, &path, 30).await {
        Ok(msg_id) => {
            let m = StoredMessage {
                id: msg_id.clone(),
                thread_id: t.id.clone(),
                from_id: st.node.id().to_string(),
                from_name: st.node.name().to_string(),
                body: String::new(),
                ts: lanchat_core::proto::now_ms(),
                mine: true,
                delivered: true,
                attachment: Some(Attachment {
                    name,
                    size,
                    path: path.to_string_lossy().to_string(),
                    sha256: lanchat_core::files::sha256_file(&path).unwrap_or_default(),
                    mime: String::new(),
                }),
            };
            st.store.save_message(&m).map_err(e2s)?;
            let _ = app.emit(
                "lc://file-sent",
                serde_json::json!({ "threadId": t.id, "message": m }),
            );
            Ok(msg_id)
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Kirim file dari path eksplisit (dipakai drag & drop dan untuk uji otomatis).
#[tauri::command]
#[allow(non_snake_case)]
async fn send_file_path(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    threadId: String,
    path: String,
) -> Result<String, String> {
    let st = state.inner().clone();
    let t = st
        .store
        .thread(&threadId)
        .map_err(e2s)?
        .ok_or_else(|| "thread tidak dikenal".to_string())?;
    if t.kind != "dm" {
        return Err("kirim file untuk sementara hanya di chat pribadi (DM)".into());
    }
    let p = std::path::PathBuf::from(&path);
    let name = p
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
    let sha = lanchat_core::files::sha256_file(&p).unwrap_or_default();

    let msg_id = st
        .node
        .send_file(&t.id, &p, 30)
        .await
        .map_err(|e| e.to_string())?;

    let m = StoredMessage {
        id: msg_id.clone(),
        thread_id: t.id.clone(),
        from_id: st.node.id().to_string(),
        from_name: st.node.name().to_string(),
        body: String::new(),
        ts: lanchat_core::proto::now_ms(),
        mine: true,
        delivered: true,
        attachment: Some(Attachment {
            name,
            size,
            path,
            sha256: sha,
            mime: String::new(),
        }),
    };
    st.store.save_message(&m).map_err(e2s)?;
    let _ = app.emit(
        "lc://file-sent",
        serde_json::json!({ "threadId": t.id, "message": m }),
    );
    Ok(msg_id)
}

/// Buka file hasil terimaan pakai aplikasi default OS.
#[tauri::command]
fn open_path(app: tauri::AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[allow(non_snake_case)] // argumen camelCase biar cocok dengan invoke() di JS
fn peers(state: State<AppState>) -> Result<Vec<PeerDto>, String> {
    Ok(peers_of(&state.node))
}

// ---------- pump event core → UI ----------

fn thread_label(store: &Store, node: &Node, id: &str) -> String {
    if let Ok(Some(t)) = store.thread(id) {
        if !t.name.is_empty() {
            return t.name;
        }
    }
    if id == BROADCAST {
        return "Broadcast LAN".into();
    }
    if is_room(id) {
        return "Grup".into();
    }
    node.peer(id)
        .map(|p| p.name)
        .unwrap_or_else(|| id[..6.min(id.len())].to_string())
}

fn ensure_thread_for(
    store: &Store,
    node: &Node,
    thread_id: &str,
    kind: &str,
    members: Vec<String>,
) -> Thread {
    if let Ok(Some(t)) = store.thread(thread_id) {
        return t;
    }
    let t = Thread {
        id: thread_id.to_string(),
        kind: kind.to_string(),
        name: thread_label(store, node, thread_id),
        members,
        last_ts: 0,
        unread: 0,
        manual_unread: false,
        preview: String::new(),
    };
    let _ = store.ensure_thread(&t);
    t
}

fn handle_event(app: &tauri::AppHandle, st: &AppState, ev: Event) {
    match ev {
        Event::Text {
            id,
            from,
            from_name,
            to,
            body,
            ts,
            mine,
            members,
        } => {
            let (thread_id, kind, mems) = if is_room(&to) {
                (to.clone(), "group", members.clone().unwrap_or_default())
            } else if to == BROADCAST {
                (BROADCAST.to_string(), "broadcast", vec![])
            } else {
                // DM: thread id = lawan bicara
                let other = if mine { to.clone() } else { from.clone() };
                (other.clone(), "dm", vec![other])
            };
            ensure_thread_for(&st.store, &st.node, &thread_id, kind, mems);
            let msg = StoredMessage {
                id,
                thread_id: thread_id.clone(),
                from_id: from,
                from_name,
                body,
                ts,
                mine,
                delivered: false,
                attachment: None,
            };
            if st.store.save_message(&msg).is_err() {
                return;
            }
            let active = st.active.lock().unwrap().as_deref() == Some(thread_id.as_str());
            if !mine && !active {
                let _ = st.store.bump_unread(&thread_id);
            }
            let thread = st.store.thread(&thread_id).ok().flatten();
            let _ = app.emit(
                "lc://msg",
                serde_json::json!({
                    "threadId": thread_id,
                    "message": msg,
                    "thread": thread,
                    "notify": !mine && !active,
                }),
            );
        }
        Event::Ack { id, ts } => {
            let _ = app.emit("lc://ack", serde_json::json!({ "messageId": id, "ts": ts }));
        }
        Event::Typing { from, to, on } => {
            let thread_id = if from == st.node.id() { to } else { from.clone() };
            let _ = app.emit(
                "lc://typing",
                serde_json::json!({ "threadId": thread_id, "on": on, "from": from }),
            );
        }
        Event::Read {
            from,
            up_to_ts,
            ..
        } => {
            let _ = st.store.mark_delivered(&from, up_to_ts);
            let _ = app.emit(
                "lc://read",
                serde_json::json!({ "threadId": from, "upToTs": up_to_ts }),
            );
        }
        Event::PeerUp(_) | Event::PeerDown(_) | Event::PeerStatus { .. } => {
            let _ = app.emit("lc://peers", serde_json::json!({ "peers": peers_of(&st.node) }));
        }
        Event::FileOffer {
            id,
            from,
            from_name,
            to,
            name,
            size,
            ts,
            mine,
            members,
        } => {
            // hanya file masuk yang perlu dicatat di thread; yang keluar sudah
            // disimpan oleh command send_file (biar tidak dobel)
            if mine {
                return;
            }
            let thread_id = from.clone();
            let _ = members;
            ensure_thread_for(&st.store, &st.node, &thread_id, "dm", vec![thread_id.clone()]);
            let msg = StoredMessage {
                id,
                thread_id: thread_id.clone(),
                from_id: from,
                from_name,
                body: String::new(),
                ts,
                mine: false,
                delivered: false,
                attachment: Some(Attachment {
                    name: name.clone(),
                    size,
                    path: String::new(),
                    sha256: String::new(),
                    mime: String::new(),
                }),
            };
            let _ = st.store.save_message(&msg);
            let _ = st.store.bump_unread(&thread_id);
            let _ = app.emit(
                "lc://file-offer",
                serde_json::json!({ "threadId": thread_id, "message": msg, "thread": st.store.thread(&thread_id).ok().flatten() }),
            );
        }
        Event::FileProgress {
            id,
            name,
            sent_bytes,
            total_bytes,
            outgoing,
        } => {
            let _ = app.emit(
                "lc://file-progress",
                serde_json::json!({
                    "id": id, "name": name, "sentBytes": sent_bytes,
                    "totalBytes": total_bytes, "outgoing": outgoing
                }),
            );
        }
        Event::FileReceived {
            id,
            from,
            from_name,
            to,
            name,
            size,
            path,
            ts,
            members,
        } => {
            let _ = to;
            let _ = members;
            let thread_id = from.clone();
            // pesan sudah dibuat saat offer; sekarang lengkapi path + sha256
            let msg = StoredMessage {
                id,
                thread_id: thread_id.clone(),
                from_id: from,
                from_name,
                body: String::new(),
                ts,
                mine: false,
                delivered: true,
                attachment: Some(Attachment {
                    name: name.clone(),
                    size,
                    path: path.clone(),
                    sha256: lanchat_core::files::sha256_file(std::path::Path::new(&path))
                        .unwrap_or_default(),
                    mime: String::new(),
                }),
            };
            let _ = st.store.save_message(&msg);
            let _ = app.emit(
                "lc://file-received",
                serde_json::json!({ "threadId": thread_id, "message": msg }),
            );
        }
        Event::FileFailed { id, name, reason } => {
            let _ = app.emit(
                "lc://file-failed",
                serde_json::json!({ "id": id, "name": name, "reason": reason }),
            );
        }
        Event::Warn(w) => {
            let _ = app.emit("lc://warn", serde_json::json!({ "message": w }));
        }
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let name = std::env::var("LANCHAT_NAME").unwrap_or_else(|_| {
                std::env::var("USER").unwrap_or_else(|_| "user".into())
            });
            let port: u16 = std::env::var("LANCHAT_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(lanchat_core::proto::DEFAULT_APP_PORT);
            let dir = std::env::var("LANCHAT_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| {
                    std::env::var("HOME")
                        .map(|h| std::path::PathBuf::from(h).join(".lanchat"))
                        .unwrap_or_else(|_| std::path::PathBuf::from("./.lanchat"))
                });

            let mut cfg = Config::new(name, dir.clone());
            cfg.port = port;
            // Fallback kalau jaringan memblok multicast: LANCHAT_SEEDS="10.10.1.31:41235,..."
            if let Ok(s) = std::env::var("LANCHAT_SEEDS") {
                cfg.seeds = s
                    .split(',')
                    .filter_map(|t| t.trim().parse().ok())
                    .collect();
            }
            // LANCHAT_DISCOVERY=0 → jangan announce ke multicast (dipakai uji terisolasi)
            if std::env::var("LANCHAT_DISCOVERY").as_deref() == Ok("0") {
                cfg.discovery = false;
            }

            let node = tauri::async_runtime::block_on(Node::start(cfg))
                .map_err(|e| format!("gagal start node: {e}"))?;
            // Judul window menyertakan nama node supaya dua instance di satu host
            // (dan skrip uji xdotool) bisa dibedakan.
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_title(&format!("LAN Chat — {}", node.name()));
            }
            let store = Store::open(&dir.join("lanchat.db"))
                .map_err(|e| format!("gagal buka riwayat: {e}"))?;

            let state: AppState = Arc::new(App {
                node,
                store: Arc::new(store),
                active: Mutex::new(None),
            });
            app.manage(state.clone());

            let handle = app.handle().clone();
            let st = state.clone();
            let mut rx = st.node.subscribe();
            tauri::async_runtime::spawn(async move {
                while let Ok(ev) = rx.recv().await {
                    handle_event(&handle, &st, ev);
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            messages,
            open_dm,
            create_group,
            send,
            typing,
            set_active,
            mark_read,
            set_unread,
            clear_chat,
            set_status,
            set_setting,
            peers,
            send_file,
            send_file_path,
            open_path
        ])
        .run(tauri::generate_context!())
        .expect("gagal menjalankan app");
}