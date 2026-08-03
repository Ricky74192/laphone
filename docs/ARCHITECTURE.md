# laphone architecture

## Goals (from IDEA.md)

- USB-only projection (stability first, no WiFi)
- Phone screen shown on PC when connected via cable — useful when PC WeChat is delayed (proxy/VPN) or while slacking off
- Standalone packaged app, not a terminal-launched tool
- Screen-off mirroring (phone display off, projection keeps running)
- Well-adapted mouse / keyboard / touchpad input

## The one physical constraint

**Screen capture happens on the phone.** There is no way around running a
capture process on the device. Three possible designs:

| Design | Install on phone | Screen-off | Input latency | Verdict |
|---|---|---|---|---|
| Normal app (MediaProjection) | APK + per-session consent dialog | Often stops on screen-off | High (adb shell input) | Rejected |
| **Pushed temporary server** (`adb push` + `app_process`, shell perms) | **None** (temp file, gone on reboot, no dialogs) | Yes (power-mode + wakelock) | Low (InputManager injection) | **Chosen for M1+** |
| Built-in tools only (`screenrecord` + `input`) | None | **No — screenrecord stops when display sleeps** (verified, see DEVICE_TESTING) | Medium | M0 PoC only |

Key facts:
- The pushed-server model is what scrcpy uses: a small jar is pushed to
  `/data/local/tmp`, run via `app_process` with shell UID. No APK, no install,
  no permission prompt; the file disappears on reboot.
- A normal app has no `INJECT_EVENTS` permission (shell does), so mouse input
  would have to go through `adb shell input` per event — too slow for dragging.
- Screen-off capture requires either shell-level power-mode control +
  wakelock (true screen-off) or brightness-0 + extended timeout (soft off).
  Some OEM ROMs stop the encoder on display power-off, so the client should
  auto-detect which mode works and fall back.

## M0 (current) — zero-install pipeline

```
phone ──screenrecord──▶ H.264 ──adb exec-out──▶ laphone (Rust)
                                                  │ OpenH264 decode
                                                  ├─▶ SDL2 window (IYUV texture)
                                                  └─▶ click ──adb shell input tap──▶ phone
```

- `adb exec-out screenrecord --output-format=h264 --size WxH --bit-rate 8M /dev/stdout`
- Decode: `openh264` crate (Cisco OpenH264, BSD; statically linked, no DLLs)
- Render: SDL2 `IYUV` streaming texture (GPU YUV→RGB), 540x1200 window from 1080x2400 phone
- Input: left click → `adb shell input tap`; mapping = window coords × scale

## M1+ — pushed temporary server

```
phone: /data/local/tmp/laphone-server  (app_process, shell UID)
  ├─ MediaCodec H.264 encode (VirtualDisplay / display capture)
  ├─ screen-off: PowerManager power-mode OFF + partial wakelock
  │               fallback: brightness 0 + timeout max (auto-detect)
  ├─ input: InputManager.injectInputEvent (low latency)  [UHID mode later]
  └─ transport: local abstract socket ← adb forward ← PC
PC: laphone client spawns adb, pushes server, manages lifecycle
```

## Input modes (M3 target)

- **Injection**: InputManager events via shell — lowest latency, default
- **UHID**: PC keyboard/mouse presented as a real HID device (kernel CONFIG_UHID)
  — passes apps that reject synthetic touches
- **Touchpad differentiation** (beyond scrcpy): two-finger pinch → real
  dual-point touch injection; two-finger scroll → inertial swipe;
  three-finger → back / recents; hover → virtual cursor

## Transport notes

- USB 2.0 (480 Mbps) is plenty: 1080p60 H.264 ≈ 10–20 Mbps
- `adb exec-out` avoids adb-client pipe translation (raw stdout)
- In Rust, spawn adb.exe and read its stdout via an OS pipe — no MSYS/Cygwin
  binary corruption issues

## Repository layout

```
src/              Rust client
deps/             SDL2 devel package (gitignored, fetched via script)
docs/             architecture + device test log
scripts/          dependency fetch / packaging
```
