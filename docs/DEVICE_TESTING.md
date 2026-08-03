# Device test log

Test methodology: `adb exec-out screenrecord --output-format=h264 --size 1080x2400 --bit-rate 8M /dev/stdout` captured to file, decoded with ffmpeg (`-f h264 -i file -f null -`).

## Xiaomi 13 — 2211133C (fuxi), HyperOS V816, Android 16

| Date | Test | Result | Notes |
|---|---|---|---|
| 2026-08-03 | Record 6s, screen **Dozing** (locked, display off) | **FAIL — 0 bytes** | screenrecord produces nothing while display is in Doze. Confirms the "screen-off breaks mirroring" problem; M0 cannot solve it, M1 server must. |
| 2026-08-03 | Record 6s, screen **Awake** (lock screen) | **PASS — 641,555 bytes** | 1080x2400 @ 8 Mbps, decodes clean. `--size 1080x2400` accepted. |
| 2026-08-03 | Same via `adb exec-out … /dev/stdout` redirect | **PASS — 473,440 bytes** | Raw H.264 Annex-B, decodes with zero errors. |
| 2026-08-03 | `adb shell input tap` while screen locked | PASS | Tap lands on lock screen (expected). Unlock behavior TBD. |
| 2026-08-03 | Continuous mirroring (laphone M0) | **PASS** | Rendered at ~15 fps (1080x2400@8M). Ran **288s continuously** — NO 180s screenrecord cap observed on this build. Exited cleanly via stall detection when the device disconnected (adb gone). Auto-restart added as defense for stream ends (future ROM caps / hiccups). |
| 2026-08-03 | OpenH264 naive chunk feeding | FAIL | Feeding 64KB chunks without NAL splitting drops SPS/PPS (dsNoParamSets). Fixed with Annex-B start-code splitter (feed complete NALs incl. start code). |
| 2026-08-03 | Offline restart harness (fake adb + ffmpeg libx264 stream) | **PASS** | No phone needed: fake `adb.exe` (gcc, `_setmode(_O_BINARY)` required — text-mode stdout corrupts H.264) cats a generated stream then exits. Verified: stream end → auto-restart → decoder resync → render continues (window title `laphone M0 — 7.0 fps` across 30+ restart cycles). libx264 streams produce transient Native:16/16384 decode errors at stream start (cosmetic; real screenrecord stream had zero errors over 288s). |
| 2026-08-03 | **Screen-off matrix** (brightness 59/auto, stayon true) | — | **A** awake 5s: 193 frames. **B** KEYCODE_POWER → Dozing 5s: 33 frames (2-3 fade-out frames then stream dies; earlier session got 0 bytes). **C** soft-off (brightness 0 → MIUI clamps to 1 + stayon) 6s: **131 frames, continuous — screenrecord survives soft-off**. Live check: laphone rendered 4.7 fps with screen visually black. **Conclusion: 真息屏 needs M1 server (wakelock + power-mode); 软息屏 works on the M0 zero-install pipeline → built as S-key "SOFT-OFF" mode (saves/restores brightness+mode+stay_on_while_plugged_in).** |
| 2026-08-03 | Stall diagnosis (repeated "phone fell asleep" events) | — | **Root cause: USB drop, not MIUI sleep.** After unplug, stayon only applies while plugged → screen dozes → screenrecord dies. Auto-wake added: on stall, `KEYCODE_WAKEUP` every 5s while adb reachable; exits only when adb unreachable. Verified offline: stays alive with shell OK, exits with shell dead. |
| 2026-08-03 | Manual gates pending (need device) | — | New letterboxed click mapping; S-key with **IME in English mode** (Chinese IME swallows ASCII keys — likely cause of earlier "S 没反应"); window resize. |
| 2026-08-03 | Window resize (SetWindowPos 600×900, 250×500) | **PASS** | No crash; render continues at both sizes (letterbox path OK). |
| 2026-08-03 | "Low fps on static screen" mystery | — | **Not a bug: screenrecord fps = display update rate.** Static black/locked screen → ~0.5 fps (frames only emitted on content change); after KEYCODE_HOME (launcher visible) → 11.3 fps instantly. Brightness was normal (65/auto). M1 server will behave the same (surface capture). |

## Next tests

- [ ] screenrecord while screen off after wake-lock workarounds (brightness 0)
- [ ] tap accuracy across window scaling
- [ ] latency measurement (frame counter vs wall clock)
