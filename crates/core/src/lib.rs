//! lanchat-core — inti dari LAN Chat (M0)
//!
//! Modul:
//! * [`proto`]     — tipe wire (announce + frame) & framing TCP
//! * [`identity`]  — keypair Ed25519 per instalasi, id + fingerprint
//! * [`discovery`] — presence via UDP multicast (dengan fallback broadcast/seed)
//! * [`node`]      — orkestrasi: peer table, koneksi TCP, event stream
//! * [`store`]     — riwayat lokal (SQLite): pesan, thread, setting

pub mod discovery;
pub mod identity;
pub mod node;
pub mod proto;
pub mod store;

pub use node::{Config, Event, Node, Peer};
pub use proto::{is_room, room_id, Status, BROADCAST};
pub use store::{Store, StoredMessage, Thread};
