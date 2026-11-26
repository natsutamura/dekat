//! dekat-core — inti dari Dekat
//!
//! Modul:
//! * [`proto`]     — tipe wire (announce + frame) & framing TCP
//! * [`identity`]  — keypair Ed25519 per instalasi, id + fingerprint
//! * [`discovery`] — presence via UDP multicast (dengan fallback broadcast/seed)
//! * [`node`]      — orkestrasi: peer table, koneksi TCP, event stream
//! * [`store`]     — riwayat lokal (SQLite): pesan, thread, setting, lampiran
//! * [`files`]     — transfer file: offer/accept/chunk/done + verifikasi sha256

pub mod discovery;
pub mod files;
pub mod identity;
pub mod node;
pub mod proto;
pub mod store;
pub mod tls;

pub use files::{FileMeta, FileTransfer, Transfer};
pub use node::{Config, Event, Node, Peer};
pub use proto::{is_room, room_id, Status, BROADCAST};
pub use store::{Attachment, Store, StoredMessage, Thread};