# laphone

**USB 有线投屏：手机插上数据线，屏幕直接出现在电脑上。息屏不断流、键鼠触摸板适配、手机端零安装。**

laphone is a Windows USB phone-mirroring tool with these priorities:

- **Plug & play** — a single exe, tray-resident; the mirror window pops up automatically when a phone is plugged in
- **Zero install on the phone** — no APK, no permission dialogs, nothing persists on the device
- **Screen-off mirroring** — the phone display can turn off while the projection keeps running
- **Input adapted** — mouse / keyboard / touchpad gestures mapped to the phone
- **USB-only** — stability first; no WiFi streaming (project decision)

## Status

**M0** — the full zero-install pipeline works: `screenrecord` → H.264 over USB → OpenH264 decode → SDL2 window, with full input mapping (click/drag/wheel/keys/text/IME), soft screen-off (Ctrl+S), resizable letterboxed window, and a background daemon that auto-opens a mirror per plugged device and closes it on unplug.
Verified on Xiaomi 13 (fuxi, HyperOS V816, Android 16). See [docs/DEVICE_TESTING.md](docs/DEVICE_TESTING.md).

## Roadmap

- **M0** (current): zero-install mirroring + full input mapping + auto start/stop daemon
- **M1**: pushed temporary server (`app_process`, shell permissions) — true screen-off mirroring (power-mode off), low-latency input injection (InputManager), real multi-touch pinch from touchpad, clipboard
- **M2**: clipboard sync, drag-and-drop files, audio (Android 11+)
- **M3**: UHID mode, multi-device polish, always-on-top mini window, boss key
- **M4**: packaging/installer, auto-update, tray icon for the daemon

## Build (Windows)

Prerequisites: Rust (GNU toolchain `x86_64-pc-windows-gnu`), `adb` in PATH.
SDL2 is built from source automatically (bundled feature, cmake + mingw32-make required).

```bash
cargo build --release
```

No DLLs to copy — SDL2 is statically linked into the exe.

## Usage

```bash
# one-shot mirror (first connected device, or pass a serial)
laphone.exe [serial]

# background daemon: watch USB, auto-open a mirror per plugged device,
# auto-close it on unplug (polls every 2s, no console windows)
laphone.exe --daemon

# register/remove Windows login autostart for the daemon
laphone.exe --install
laphone.exe --uninstall
```

### Input mapping

| PC input | Phone action |
|---|---|
| left press + drag | touch DOWN / MOVE / UP (tap = quick click) |
| right click | back |
| middle click | home |
| mouse wheel | scroll (swipe at cursor) |
| text keys | typed text (incl. IME composition) |
| Enter / Backspace / Tab / arrows / PgUp / PgDn / Home / End / Del / Esc | corresponding Android keys (Esc = back) |
| Ctrl+S | soft screen-off (phone screen black, projection keeps running; press again to restore) |
| Ctrl+Q / window close | quit |

Note: if a Chinese IME is active in Chinese mode, letters go to the IME — switch it to English mode for key shortcuts like Ctrl+S.

Known M0 limits (→ M1): real pinch zoom needs multi-touch injection (server-side); `input` command latency is ~20-40 ms per gesture (server injection will be sub-ms).

## Architecture

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the design decisions (why a pushed temporary server, how screen-off mirroring works, input modes).

## License

Apache-2.0
