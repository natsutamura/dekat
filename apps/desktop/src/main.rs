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
    /// Avatar sendiri sebagai data URL, siap dipakai di `<img src>`.
    avatar: Option<String>,
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
    avatar: Option<String>,
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
            avatar: p.avatar.map(|a| {
                if a.is_empty() { String::new() } else { format!("data:image/png;base64,{a}") }
            }),
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
        avatar: node
            .avatar()
            .filter(|a| !a.is_empty())
            .map(|a| format!("data:image/png;base64,{a}")),
    }
}

/// Ganti nama tampilan (bukan nama PC). Berlaku langsung ke semua peer.
#[tauri::command]
fn set_display_name(state: State<AppState>, name: String) -> Result<Me, String> {
    state
        .node
        .set_profile(Some(&name), None)
        .map_err(e2s)?;
    Ok(me_of(&state.node))
}

/// Pasang avatar dari PNG base64 yang sudah diperkecil di klien.
#[tauri::command]
fn set_my_avatar(state: State<AppState>, dataBase64: String) -> Result<Me, String> {
    const MAX: usize = 512 * 1024; // 512 KB cukup untuk 128x128 PNG
    if dataBase64.len() > MAX {
        return Err("avatar terlalu besar (maks 512 KB setelah diperkecil)".into());
    }
    let raw = dataBase64.rsplit(',').next().unwrap_or("").to_string();
    if raw.is_empty() {
        return Err("data avatar kosong".into());
    }
    state
        .node
        .set_profile(None, Some(raw))
        .map_err(e2s)?;
    Ok(me_of(&state.node))
}

/// Hapus avatar, kembali ke inisial nama.
#[tauri::command]
fn clear_my_avatar(state: State<AppState>) -> Result<Me, String> {
    state
        .node
        .set_profile(None, Some(String::new()))
        .map_err(e2s)?;
    Ok(me_of(&state.node))
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
                read: false,
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
        read: false,
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

// ---------- voice call: signalling ----------
// Audio-nya sendiri jalan di webview (WebRTC). Rust cuma meneruskan SDP/ICE
// lewat koneksi TCP yang sudah dipakai chat.

#[tauri::command]
#[allow(non_snake_case)]
fn call_offer(
    state: State<AppState>,
    peerId: String,
    callId: String,
    sdp: String,
    members: Option<Vec<String>>,
) -> Result<(), String> {
    let members = members.unwrap_or_default();
    state
        .node
        .call_offer_group(&peerId, &callId, &sdp, &members)
        .map_err(e2s)
}

#[tauri::command]
#[allow(non_snake_case)]
fn call_answer(
    state: State<AppState>,
    peerId: String,
    callId: String,
    accept: bool,
    sdp: String,
    reason: Option<String>,
) -> Result<(), String> {
    state
        .node
        .call_answer(&peerId, &callId, accept, &sdp, reason.as_deref().unwrap_or(""))
        .map_err(e2s)
}

#[tauri::command]
#[allow(non_snake_case)]
fn call_ice(
    state: State<AppState>,
    peerId: String,
    callId: String,
    candidate: String,
    sdpMid: Option<String>,
    sdpMlineIndex: Option<u32>,
) -> Result<(), String> {
    state
        .node
        .call_ice(
            &peerId,
            &callId,
            &candidate,
            sdpMid.as_deref().unwrap_or("0"),
            sdpMlineIndex.unwrap_or(0),
        )
        .map_err(e2s)
}

#[tauri::command]
#[allow(non_snake_case)]
fn call_end(state: State<AppState>, peerId: String, callId: String, reason: Option<String>) -> Result<(), String> {
    state
        .node
        .call_end(&peerId, &callId, reason.as_deref().unwrap_or("ditutup"))
        .map_err(e2s)
}

/// Notifikasi native saat pesan masuk dan jendela tidak fokus.
///
/// `request_user_attention` yang membuat tombol taskbar berkedip di Windows
/// (Critical = kedip terus sampai diklik; Informational = sekali).
fn notify_incoming(app: &tauri::AppHandle, msg: &StoredMessage, thread_id: &str) {
    use tauri_plugin_notification::NotificationExt;

    let body = if msg.body.trim().is_empty() {
        match &msg.attachment {
            Some(a) => format!("📎 {}", a.name),
            None => "(kosong)".to_string(),
        }
    } else {
        msg.body.chars().take(140).collect()
    };

    let _ = app
        .notification()
        .builder()
        .title(&msg.from_name)
        .body(&body)
        .show();

    // Kedipkan entri taskbar (Windows) / minta perhatian WM (Linux).
    if let Some(win) = app.get_webview_window("main") {
        let focused = win.is_focused().unwrap_or(true);
        if !focused {
            let _ = win.request_user_attention(Some(tauri::UserAttentionType::Informational));
        }
        // Judul ikut menandai pesan belum dibaca, biar kelihatan di taskbar.
        let _ = thread_id;
    }
}

/// Ambil gambar dari clipboard OS, simpan ke file, kembalikan path-nya.
///
/// Kenapa di Rust, bukan lewat event `paste` di webview: WebKitGTK tidak
/// menyerahkan gambar clipboard sebagai file di `clipboardData`, jadi handler
/// JavaScript tidak pernah menerima datanya. Membaca langsung dari OS juga
/// membuat perilakunya sama di Windows dan Linux.
///
/// `Ok(None)` = clipboard tidak berisi gambar (biar JS bisa fallback ke paste teks).
#[tauri::command]
fn paste_image_from_clipboard(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let mut cb = match arboard::Clipboard::new() {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    let img = match cb.get_image() {
        Ok(i) => i,
        Err(_) => return Ok(None), // clipboard berisi teks, bukan gambar
    };

    let dir = state.inner().node.files_dir().join("paste");
    std::fs::create_dir_all(&dir).map_err(e2s)?;

    // PNG supaya lossless dan bisa langsung ditampilkan inline oleh penerima.
    let path = dir.join(format!("paste-{}.png", lanchat_core::proto::now_ms()));
    let file = std::fs::File::create(&path).map_err(e2s)?;
    let mut buf = std::io::BufWriter::new(file);
    {
        let mut enc = png::Encoder::new(&mut buf, img.width as u32, img.height as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(e2s)?;
        writer.write_image_data(&img.bytes).map_err(e2s)?;
    }
    Ok(Some(path.to_string_lossy().to_string()))
}

/// Simpan gambar dari clipboard (base64) ke file sementara, kembalikan path-nya.
///
/// Dipakai untuk paste gambar (Ctrl+V) di kotak pesan: webview memberi data URL,
/// lalu file itu dikirim lewat jalur file biasa (`send_file_path`).
#[tauri::command]
fn save_pasted_image(
    state: State<'_, AppState>,
    data_base64: String,
    ext: Option<String>,
) -> Result<String, String> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    const MAX: usize = 24 * 1024 * 1024;

    if data_base64.len() > MAX {
        return Err("gambar terlalu besar (maks 24 MB)".into());
    }
    let bytes = B64.decode(data_base64.as_bytes()).map_err(|e| format!("data tidak valid: {e}"))?;

    // Ekstensi ditentukan dari magic bytes, bukan dari nama klaim klien.
    let ext = match bytes.as_slice() {
        b if b.starts_with(&[0x89, b'P', b'N', b'G']) => "png",
        b if b.starts_with(&[0xFF, 0xD8, 0xFF]) => "jpg",
        b if b.starts_with(b"GIF8") => "gif",
        b if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" => "webp",
        _ => ext.as_deref().unwrap_or("png"),
    };

    let dir = state.inner().node.files_dir().join("paste");
    std::fs::create_dir_all(&dir).map_err(e2s)?;
    let name = format!("paste-{}.{ext}", lanchat_core::proto::now_ms());
    let path = dir.join(&name);
    std::fs::write(&path, &bytes).map_err(e2s)?;
    Ok(path.to_string_lossy().to_string())
}

/// Baca file gambar lokal → data URL, supaya bisa ditampilkan inline di bubble.
/// Dibatasi 12 MB: gambar besar lebih baik dibuka lewat tombol "Buka".
#[tauri::command]
fn read_image(path: String) -> Result<String, String> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    const MAX: u64 = 12 * 1024 * 1024;

    let p = std::path::Path::new(&path);
    let meta = std::fs::metadata(p).map_err(|e| format!("tidak bisa baca {}: {e}", p.display()))?;
    if meta.len() > MAX {
        return Err("gambar terlalu besar untuk ditampilkan inline".into());
    }
    let bytes = std::fs::read(p).map_err(|e| e.to_string())?;

    // Tebak MIME dari magic bytes, bukan dari ekstensi (nama file dari jaringan tidak dipercaya).
    let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        return Err("bukan gambar yang didukung (png/jpg/gif/webp)".into());
    };
    Ok(format!("data:{mime};base64,{}", B64.encode(&bytes)))
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
                read: false,
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
            let notify = !mine && !active;
            if notify {
                // Notifikasi native + kedipkan taskbar. Ini yang bikin user tahu
                // ada pesan walau app sedang di belakang.
                notify_incoming(&app, &msg, &thread_id);
            }
            let _ = app.emit(
                "lc://msg",
                serde_json::json!({
                    "threadId": thread_id,
                    "message": msg,
                    "thread": thread,
                    "notify": notify,
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
            let _ = st.store.mark_receipt_read(&from, up_to_ts);
            let _ = app.emit(
                "lc://read",
                serde_json::json!({ "threadId": from, "upToTs": up_to_ts }),
            );
        }
        Event::PeerUp(_)
        | Event::PeerDown(_)
        | Event::PeerStatus { .. }
        | Event::PeerProfile { .. } => {
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
                read: false,
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
            to: _,
            name,
            size,
            path,
            ts,
            members,
        } => {
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
                read: false,
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
        Event::CallIncoming {
            call_id,
            from,
            from_name,
            sdp,
            members,
        } => {
            let _ = app.emit(
                "lc://call-incoming",
                serde_json::json!({
                    "callId": call_id, "from": from, "fromName": from_name,
                    "sdp": sdp, "members": members,
                }),
            );
        }
        Event::CallAccepted { call_id, from, sdp } => {
            let _ = app.emit(
                "lc://call-accepted",
                serde_json::json!({ "callId": call_id, "from": from, "sdp": sdp }),
            );
        }
        Event::CallRejected {
            call_id,
            from,
            reason,
        } => {
            let _ = app.emit(
                "lc://call-rejected",
                serde_json::json!({ "callId": call_id, "from": from, "reason": reason }),
            );
        }
        Event::CallIce {
            call_id,
            from,
            candidate,
            sdp_mid,
            sdp_mline_index,
        } => {
            let _ = app.emit(
                "lc://call-ice",
                serde_json::json!({
                    "callId": call_id, "from": from, "candidate": candidate,
                    "sdpMid": sdp_mid, "sdpMlineIndex": sdp_mline_index
                }),
            );
        }
        Event::CallEnded {
            call_id,
            from,
            reason,
        } => {
            let _ = app.emit(
                "lc://call-ended",
                serde_json::json!({ "callId": call_id, "from": from, "reason": reason }),
            );
        }
        Event::Warn(w) => {
            let _ = app.emit("lc://warn", serde_json::json!({ "message": w }));
        }
    }
}

/// Nama default: `LANCHAT_NAME` → USERNAME (Windows) / USER (Unix) → "user".
fn default_name() -> String {
    for key in ["USERNAME", "USER"] {
        if let Ok(v) = std::env::var(key) {
            if !v.trim().is_empty() {
                return v;
            }
        }
    }
    "user".into()
}

/// Folder data default. Di Windows pakai %APPDATA%\LANChat, di Unix ~/.lanchat.
fn default_data_dir() -> std::path::PathBuf {
    if cfg!(windows) {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return std::path::PathBuf::from(appdata).join("LANChat");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return std::path::PathBuf::from(home).join(".lanchat");
    }
    // fallback terakhir: folder di samping executable
    std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(".lanchat")
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let name = std::env::var("LANCHAT_NAME").unwrap_or_else(|_| default_name());
            let port: u16 = std::env::var("LANCHAT_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(lanchat_core::proto::DEFAULT_APP_PORT);
            let dir = std::env::var("LANCHAT_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| default_data_dir());

            let mut cfg = Config::new(name, dir.clone());
            cfg.port = port;
            // Default: hanya peer yang mendukung TLS. Dinyalakan lewat env kalau
            // memang harus bicara dengan build lama (risiko penyadapan diterima).
            cfg.allow_plaintext = std::env::var("LANCHAT_ALLOW_PLAINTEXT").is_ok();
            // LANCHAT_SEED="192.168.1.20:41234,192.168.1.21:41234" untuk jaringan
            // yang memblok multicast (announce dikirim unicast ke alamat ini juga).
            if let Ok(seeds) = std::env::var("LANCHAT_SEED") {
                cfg.seeds = seeds
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
            }
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
            set_display_name,
            set_my_avatar,
            clear_my_avatar,
            set_setting,
            peers,
            send_file,
            send_file_path,
            open_path,
            read_image,
            save_pasted_image,
            paste_image_from_clipboard,
            call_offer,
            call_answer,
            call_ice,
            call_end
        ])
        .run(tauri::generate_context!())
        .expect("gagal menjalankan app");
}