# UI Spec — "iMessage di LAN"

Referensi: screenshot macOS Messages yang dikirim ray (sidebar kiri + panel percakapan kanan,
bubble biru untuk pesan sendiri, abu untuk lawan bicara).

## Layout

```
┌───────────────┬──────────────────────────────────────┐
│ sidebar 300px │ header thread 48px  (avatar+nama)    │
│  search 36px  ├──────────────────────────────────────┤
│  thread rows  │ messages (scroll, bubble rata kanan/ │
│  64px each    │ kiri, date separator di tengah)      │
│               ├──────────────────────────────────────┤
│               │ chat bar 56px (attach│input pill│emoji│send) │
└───────────────┴──────────────────────────────────────┘
```

- Window minimum 900×560, default 1080×660. Sidebar bisa ditutup di layar sempit (< 720px → single pane).
- Mobile (Android): sidebar jadi layar daftar chat, thread jadi layar penuh, tombol back di header.

## Design token

| token | light | dark |
|---|---|---|
| `--bg-window` | `#FFFFFF` | `#1E1E1E` |
| `--bg-sidebar` | `#F4F4F5` | `#252527` |
| `--bg-sidebar-selected` | `#0A84FF` (teks putih) | `#0A84FF` |
| `--bg-bubble-out` | `#0B84FF` (teks putih) | `#0B84FF` |
| `--bg-bubble-in` | `#E9E9EB` (teks `#000`) | `#3A3A3C` (teks `#FFF`) |
| `--text-primary` | `#000000` | `#F5F5F7` |
| `--text-secondary` | `#8E8E93` | `#98989D` |
| `--border` | `#E0E0E2` | `#3A3A3C` |
| `--accent` | `#0A84FF` | `#0A84FF` |
| `--status-online` | `#34C759` | sama |
| `--status-idle` | `#FF9F0A` | sama |
| `--status-dnd` | `#FF3B30` | sama |
| `--status-offline` | `#8E8E93` | sama |

Bubble:
- radius 18px; sudut ekor: keluar `border-bottom-right-radius: 4px`, masuk `border-bottom-left-radius: 4px`
- max-width 70% / 520px, padding 8px 12px, line-height 1.35, font 14px
- jarak antar bubble dalam satu grup 2px, antar grup 8px
- timestamp 11px `--text-secondary`, muncul di bawah grup terakhir atau saat hover
- status "Delivered/✓" 10px di bawah bubble terakhir sendiri

Tipografi: `-apple-system, "Segoe UI Variable", "Segoe UI", Roboto, "Noto Sans", sans-serif`.

## Komponen

| komponen | detail |
|---|---|
| `Sidebar` | search field, list thread: avatar 36px, nama 14px semibold, preview 12px, waktu kanan atas, badge unread biru |
| `ThreadRow` | state: normal / hover `#E5E5E7` / active accent / unread (nama bold + dot biru) |
| `ChatHeader` | avatar + nama + status text ("Online", "Idle", "Do Not Disturb"), tombol call (📞) & info (ⓘ) |
| `MessageList` | virtualized scroll, sticky date separator ("Hari ini", "Kemarin", tanggal), bubble grouping per pengirim + 60 detik |
| `Bubble` | teks, gambar, file card (ikon + nama + ukuran + progress bar), emoji besar (jika pesan ≤3 emoji → 32px tanpa bubble) |
| `ChatBar` | tombol attach (lingkaran 28px), input pill (radius 18px, placeholder nama thread), emoji, tombol kirim 28px muncul saat ada teks (↑ putih di lingkaran accent) |
| `Avatar` | foto atau inisial (bg warna dari hash nama), ring 2px warna status kalau online |
| `StatusDot` | 10px + ring 2px warna background; posisi kanan-bawah avatar |
| `AvatarGroup` | group picture: stacked 2–4 avatar mini atau mosaic 2×2 |
| `EmptyState` | ikon + "Pilih percakapan" saat belum ada thread aktif |

## State & interaksi

- **Tema**: dark/light + ikuti OS (`prefers-color-scheme`); switch 180 ms.
- **Kustomisasi bubble**: pengaturan → pilih warna bubble keluar (6 preset + color picker), radius (4/12/18/22), font size (13/14/16). Disimpan lokal.
- **Mark read/unread**: klik kanan thread → toggle; unread = nama bold + badge biru; badge dihitung dari pesan masuk yang belum dibuka.
- **Clear chat**: konfirmasi dialog → hapus pesan lokal untuk thread itu, thread tetap ada.
- **Status**: online (ada aktivitas < 2 menit), idle (tidak ada input 5 menit), dnd (manual, announce tetap jalan tapi penerima menandai jangan popup), offline (TTL habis).
- **Notifikasi**: native OS saat app tidak fokus; popup in-app (toast kanan atas) saat fokus di thread lain; klik → buka thread.
- **Animasi**: bubble masuk `translateY(4px)+opacity` 120 ms ease-out; thread baru di sidebar: highlight biru pudar 800 ms.
- **Keyboard**: Enter kirim, Shift+Enter baris baru, Esc tutup dialog, Ctrl/Cmd+K cari, Ctrl/Cmd+F fokus search.

## Aksesibilitas

- Kontras teks ≥ 4.5:1 (bubble biru tetap putih, abu `#E9E9EB` pakai teks hitam).
- Status tidak hanya warna: ada label teks di header.
- Semua aksi punya keyboard path; target sentuh Android ≥ 44px.
- `prefers-reduced-motion` → matikan animasi slide.
