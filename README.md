# laphone

**USB 有线投屏：手机插上数据线，屏幕直接出现在电脑上。息屏不断流、键鼠触摸板适配、手机端零安装。**

laphone is a Windows USB phone-mirroring tool with these priorities:

- **Plug & play** — a single exe, tray-resident; the mirror window pops up automatically when a phone is plugged in
- **Zero install on the phone** — no APK, no permission dialogs, nothing persists on the device
- **Screen-off mirroring** — the phone display can turn off while the projection keeps running
- **Input adapted** — mouse / keyboard / touchpad gestures mapped to the phone
- **USB-only** — stability first; no WiFi streaming (project decision)

## Status

**M0 PoC** — the full zero-install pipeline works: `screenrecord` → H.264 over USB → OpenH264 decode → SDL2 window, plus click-to-tap injection.
Verified on Xiaomi 13 (fuxi, HyperOS V816, Android 16). See [docs/DEVICE_TESTING.md](docs/DEVICE_TESTING.md).

Not yet ready for users. No releases yet.

## Roadmap

- **M0** (current): mirror + basic tap injection via the zero-install `screenrecord`/`input` pipeline
- **M1**: pushed temporary server (`app_process`, shell permissions) — screen-off mirroring, low-latency input injection, clipboard
- **M2**: keyboard, scrolling, clipboard sync, drag-and-drop files
- **M3**: touchpad multi-touch gestures, UHID mode, audio (Android 11+), multi-device
- **M4**: packaging, auto-update, UI polish (always-on-top mini window, boss key)

## Build (Windows)

Prerequisites: Rust (GNU toolchain `x86_64-pc-windows-gnu`), `adb` in PATH.
SDL2 is built from source automatically (bundled feature, cmake + mingw32-make required).

```bash
cargo build --release
```

No DLLs to copy — SDL2 is statically linked into the exe.

## Usage (M0)

```bash
adb devices          # phone must show as "device"
target/release/laphone.exe [serial]   # click = tap; ESC = quit
```

## Architecture

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the design decisions (why a pushed temporary server, how screen-off mirroring works, input modes).

## License

Apache-2.0
