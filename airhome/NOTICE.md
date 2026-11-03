# Notices

airhome is free software, licensed under the **GNU General Public License, version 3 or (at your option) any
later version** (GPL-3.0-or-later). The full text is in [`LICENSE`](LICENSE).

## airplay2-rs

airhome is built on **airplay2-rs** (AirPlay 2 sender implementation in Rust),
<https://github.com/lmcgartland/airplay2-rs>, licensed GPL-3.0-or-later. airhome links against modified
copies of its crates (`airplay-core`, `airplay-client`, `airplay-audio`, `airplay-timing`, and their
dependencies in that workspace). The modifications made for airhome:

| File | Change |
|---|---|
| `crates/airplay-audio/src/rtp.rs` | use `socket2` instead of Unix-only `libc` calls so the crate builds on Windows |
| `crates/airplay-client/src/connection.rs` | `start_streaming_live` enables PTP sync mode (PT=87) when a PTP clock id is known, so live streams are audible |
| `crates/airplay-audio/src/streamer.rs`, `lib.rs` | `LIVE_PREFILL_PERCENT` (public setting) controls the live jitter-buffer prefill, i.e. latency |
| `crates/airplay-timing/src/ptp.rs`, `ntp.rs` | the background sync loops stop when their stop channel is dropped, instead of spinning |
| docs, examples and tests | example IP addresses and device IDs replaced with neutral placeholders (`192.168.1.x`, `AA:BB:CC:DD:EE:FF`, `00:11:22:33:44:55`) in docs, examples and test data; no behaviour change |

The modified source is distributed together with airhome, as the GPL requires.

## Other components

airhome also uses third-party Rust crates (egui/eframe, tokio, cpal, tray-icon, serde, windows, and others)
under their own permissive licences (MIT, Apache-2.0, and similar). See each crate's metadata, or run
`cargo license`.

"AirPlay", "HomePod" and "Apple" are trademarks of Apple Inc. airhome is an independent project and is not
affiliated with or endorsed by Apple.
