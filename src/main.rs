// laphone M0 — zero-install USB mirroring pipeline:
//   adb exec-out screenrecord --output-format=h264 → openh264 decode → SDL2 window
// Input (all via adb shell, shell-UID injection):
//   left drag  → input motionevent DOWN/MOVE/UP (tap = quick down+up)
//   right click→ KEYCODE_BACK ; middle click → KEYCODE_HOME
//   wheel      → input swipe (scroll) ; Ctrl+S → soft screen-off
//   keys       → input keyevent (nav/control) ; text → input text (incl. IME)
//   Ctrl+Q / window close → quit ; ESC → back
//
// Modes:
//   laphone.exe [serial]   — mirror window for one device (serial optional:
//                            defaults to the only connected device)
//   laphone.exe --daemon   — background watcher: polls `adb devices`, opens a
//                            mirror per plugged device, closes it on unplug
//   laphone.exe --install / --uninstall — register/remove autostart (Run key)
//
// Design notes:
// - Reader thread pushes raw stream chunks over an mpsc channel so the SDL
//   event loop never blocks on the adb pipe.
// - The reader thread restarts the recorder whenever the stream ends (device
//   sleep, disconnect, or a future ROM's recording cap); the decoder resyncs
//   on the fresh SPS/PPS of the new stream.
// - OpenH264 strides are padded (e.g. 1080 → 1088), but SDL IYUV textures need
//   exact pitches, so each frame is repacked into tight Y/U/V planes.
#![windows_subsystem = "windows"]

use std::collections::HashMap;
use std::env;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use openh264::decoder::Decoder;
use openh264::formats::YUVSource;
use sdl2::event::Event;
use sdl2::keyboard::{Keycode, Mod};
use sdl2::mouse::MouseButton;
use sdl2::pixels::PixelFormatEnum;
use sdl2::rect::Rect;

const PHONE_W: u32 = 1080; // TODO: auto-detect via `adb shell wm size`
const PHONE_H: u32 = 2400;
const BITRATE: &str = "8M";

/// Suppress the console window of spawned console-subsystem children (adb,
/// reg) when this process is a GUI-subsystem binary without a console —
/// otherwise every `adb devices` poll (daemon: 2s) flashes a black window.
fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// Letterboxed view rect + scale for the given window size (keeps phone aspect).
fn view_rect(win_w: u32, win_h: u32) -> (Rect, f32) {
    let scale = (win_w as f32 / PHONE_W as f32).min(win_h as f32 / PHONE_H as f32);
    let dw = (PHONE_W as f32 * scale) as i32;
    let dh = (PHONE_H as f32 * scale) as i32;
    let dx = (win_w as i32 - dw) / 2;
    let dy = (win_h as i32 - dh) / 2;
    (Rect::new(dx, dy, dw as u32, dh as u32), scale)
}

/// Map window coords to phone coords inside the letterboxed view; None if outside.
fn map_to_phone(x: i32, y: i32, win_w: u32, win_h: u32) -> Option<(i32, i32)> {
    let (dst, scale) = view_rect(win_w, win_h);
    if x >= dst.x()
        && x < dst.x() + dst.width() as i32
        && y >= dst.y()
        && y < dst.y() + dst.height() as i32
    {
        let px = ((x - dst.x()) as f32 / scale).clamp(0.0, PHONE_W as f32 - 1.0) as i32;
        let py = ((y - dst.y()) as f32 / scale).clamp(0.0, PHONE_H as f32 - 1.0) as i32;
        Some((px, py))
    } else {
        None
    }
}

fn spawn_recorder(serial: Option<&str>) -> std::io::Result<Child> {
    let mut cmd = Command::new("adb");
    if let Some(s) = serial {
        cmd.arg("-s").arg(s);
    }
    no_window(&mut cmd)
        .arg("exec-out")
        .arg("screenrecord")
        .arg("--output-format=h264")
        .arg("--bit-rate")
        .arg(BITRATE)
        .arg("--size")
        .arg(format!("{PHONE_W}x{PHONE_H}"))
        .arg("/dev/stdout")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
}

/// Map an SDL keycode to an Android keycode (nav/control keys only; printable
/// text flows through SDL_TEXTINPUT → `input text` instead).
fn keycode_to_android(k: Keycode) -> Option<i32> {
    Some(match k {
        Keycode::Escape => 4,                     // KEYCODE_BACK
        Keycode::Return | Keycode::KpEnter => 66, // KEYCODE_ENTER
        Keycode::Backspace => 67,                 // KEYCODE_DEL
        Keycode::Tab => 61,                       // KEYCODE_TAB
        Keycode::Space => 62,                     // KEYCODE_SPACE
        Keycode::Delete => 112,                   // KEYCODE_FORWARD_DEL
        Keycode::Home => 3,                       // KEYCODE_HOME
        Keycode::End => 123,                      // KEYCODE_MOVE_END
        Keycode::PageUp => 92,                    // KEYCODE_PAGE_UP
        Keycode::PageDown => 93,                  // KEYCODE_PAGE_DOWN
        Keycode::Up => 19,                        // KEYCODE_DPAD_UP
        Keycode::Down => 20,                      // KEYCODE_DPAD_DOWN
        Keycode::Left => 21,                      // KEYCODE_DPAD_LEFT
        Keycode::Right => 22,                     // KEYCODE_DPAD_RIGHT
        _ => return None,
    })
}

/// `adb shell input keyevent <code>`
fn keyevent(serial: Option<&str>, code: i32) {
    let _ = shell(serial, &format!("input keyevent {code}"));
}

/// `adb shell input motionevent <ACTION> x y` — DOWN / MOVE / UP (tap + drag).
fn motion(serial: Option<&str>, action: &str, x: i32, y: i32) {
    let _ = shell(serial, &format!("input motionevent {action} {x} {y}"));
}

/// `adb shell input text <t>` — with `%s` escaping for spaces.
fn text_input(serial: Option<&str>, t: &str) {
    let _ = shell(serial, &format!("input text {}", t.replace(' ', "%s")));
}

/// Run an adb shell command, return trimmed stdout (None on failure).
fn shell(serial: Option<&str>, cmd: &str) -> Option<String> {
    let mut c = Command::new("adb");
    if let Some(s) = serial {
        c.arg("-s").arg(s);
    }
    let out = no_window(&mut c).arg("shell").arg(cmd).output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

/// Soft screen-off: brightness 0 + stay-awake. The display pipeline keeps
/// running (verified: screenrecord survives), so mirroring continues while
/// the screen looks black. Real power-off needs the M1 server (wakelock).
/// Saves prior display state and restores it when toggled off / on exit.
fn toggle_soft_off(
    serial: Option<&str>,
    on: &mut bool,
    saved: &mut Option<(String, String, String)>,
) {
    if !*on {
        let brightness = shell(serial, "settings get system screen_brightness").unwrap_or_default();
        let mode = shell(serial, "settings get system screen_brightness_mode").unwrap_or_default();
        let stay_on =
            shell(serial, "settings get global stay_on_while_plugged_in").unwrap_or_default();
        *saved = Some((brightness, mode, stay_on));
        let _ = shell(serial, "svc power stayon true");
        let _ = shell(serial, "settings put system screen_brightness_mode 0");
        let _ = shell(serial, "settings put system screen_brightness 0");
        *on = true;
    } else if let Some((b, m, s)) = saved.take() {
        let _ = shell(
            serial,
            &format!("settings put system screen_brightness_mode {m}"),
        );
        let _ = shell(
            serial,
            &format!("settings put system screen_brightness {b}"),
        );
        // "null" (unset) → restore as 0 (off)
        let stay = if s.is_empty() || s == "null" {
            "0".to_string()
        } else {
            s
        };
        let _ = shell(
            serial,
            &format!("settings put global stay_on_while_plugged_in {stay}"),
        );
        *on = false;
    }
}

/// Positions of Annex-B start codes (00 00 01 / 00 00 00 01), pointing at the
/// first byte OF the code. NAL units are fed to the decoder WITH their start
/// code (OpenH264 accepts Annex-B input), so that after draining, `pending[0]`
/// is always a start code and no NAL is ever orphaned.
fn nal_starts(data: &[u8]) -> Vec<usize> {
    let mut v = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 {
            if data[i + 2] == 1 {
                v.push(i);
                i += 3;
            } else if i + 4 <= data.len() && data[i + 2] == 0 && data[i + 3] == 1 {
                v.push(i);
                i += 4;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    v
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match env::args().nth(1).as_deref() {
        Some("--daemon") => daemon_main(),
        Some("--install") => autostart(true),
        Some("--uninstall") => autostart(false),
        serial => mirror_main(serial.map(str::to_string)),
    }
}

/// Poll `adb devices`, keep a mirror per plugged device, stop it on unplug.
fn daemon_main() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("laphone daemon: watching for USB devices…");
    let mut mirrors: HashMap<String, Child> = HashMap::new();
    loop {
        let devices = adb_devices();
        for d in &devices {
            if !mirrors.contains_key(d) {
                eprintln!("device {d} — starting mirror");
                match no_window(&mut Command::new(env::current_exe()?))
                    .arg(d)
                    .spawn()
                {
                    Ok(c) => {
                        mirrors.insert(d.clone(), c);
                    }
                    Err(e) => eprintln!("mirror spawn failed: {e}"),
                }
            }
        }
        let gone: Vec<String> = mirrors
            .iter()
            .filter(|(s, _)| !devices.contains(*s))
            .map(|(s, _)| s.clone())
            .collect();
        for g in gone {
            eprintln!("device {g} unplugged — stopping mirror");
            if let Some(mut c) = mirrors.remove(&g) {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
        let exited: Vec<String> = mirrors
            .iter_mut()
            .filter_map(|(s, c)| match c.try_wait() {
                Ok(Some(_)) => Some(s.clone()),
                _ => None,
            })
            .collect();
        for s in exited {
            eprintln!("mirror {s} exited on its own");
            mirrors.remove(&s);
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// `adb devices` → serials in "device" state.
fn adb_devices() -> Vec<String> {
    let out = match no_window(&mut Command::new("adb")).arg("devices").output() {
        Ok(o) => o.stdout,
        Err(_) => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&out);
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let serial = it.next()?;
            if it.next() == Some("device") {
                Some(serial.to_string())
            } else {
                None
            }
        })
        .collect()
}

/// Register/remove the HKCU Run key so the daemon starts at login.
fn autostart(install: bool) -> Result<(), Box<dyn std::error::Error>> {
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    if install {
        let value = format!("\"{}\" --daemon", env::current_exe()?.display());
        let st = no_window(&mut Command::new("reg"))
            .args(["add", key, "/v", "laphone", "/d", &value, "/f"])
            .status()?;
        if st.success() {
            eprintln!("autostart installed: {value}");
        } else {
            eprintln!("autostart install failed (reg add exit {:?})", st.code());
        }
    } else {
        let st = no_window(&mut Command::new("reg"))
            .args(["delete", key, "/v", "laphone", "/f"])
            .status()?;
        if st.success() {
            eprintln!("autostart removed");
        } else {
            eprintln!("autostart remove failed (reg delete exit {:?})", st.code());
        }
    }
    Ok(())
}

fn mirror_main(serial: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    // reader thread: adb pipe → channel; restarts screenrecord on stream end
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let serial_thread = serial.clone();
    std::thread::spawn(move || {
        loop {
            let mut child = spawn_recorder(serial_thread.as_deref()).expect("spawn adb");
            let mut stream = child.stdout.take().expect("adb stdout");
            let mut chunk = vec![0u8; 65536];
            let mut clean_end = false;
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => {
                        clean_end = true;
                        break;
                    }
                    Err(_) => break,
                    Ok(n) => {
                        if tx.send(chunk[..n].to_vec()).is_err() {
                            let _ = child.kill();
                            let _ = child.wait(); // reap
                            return; // main loop gone
                        }
                    }
                }
            }
            let _ = child.wait(); // reap the ended recorder
            if !clean_end {
                return;
            }
            eprintln!("recorder ended — restarting…");
            std::thread::sleep(Duration::from_millis(500));
        }
    });

    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    // initial size: fit ~80% of the primary display height, phone aspect;
    // the window is resizable and the view is letterboxed
    let bounds = video.display_bounds(0)?;
    let init_h = (bounds.height() as u32).saturating_mul(4) / 5;
    let init_w = init_h * PHONE_W / PHONE_H;
    let window = video
        .window("laphone M0", init_w, init_h)
        .position_centered()
        .resizable()
        .build()?;
    let mut canvas = window.into_canvas().accelerated().present_vsync().build()?;
    let tc = canvas.texture_creator();
    let mut texture = tc.create_texture_streaming(PixelFormatEnum::IYUV, PHONE_W, PHONE_H)?;
    let mut events = sdl.event_pump()?;

    let mut decoder = Decoder::new()?;
    let mut pending: Vec<u8> = Vec::with_capacity(1 << 20); // Annex-B accumulator
    let mut tight: Vec<u8> = Vec::new(); // reusable tight-packed YUV plane buffer
    let mut decode_errors: u32 = 0;

    let mut frames: u32 = 0;
    let started = Instant::now();
    let mut last_data = Instant::now();
    let mut title_t = Instant::now();
    let mut soft_off = false;
    let mut saved_display: Option<(String, String, String)> = None;
    let mut drag: Option<(i32, i32)> = None; // active pointer (phone coords)

    'main: loop {
        let mouse = events.mouse_state(); // owned snapshot (poll_iter borrows events mutably)
        let (win_w, win_h) = canvas.output_size().unwrap_or((init_w, init_h));
        for ev in events.poll_iter() {
            match ev {
                Event::Quit { .. } => break 'main,
                // Ctrl+Q → quit
                Event::KeyDown {
                    keycode: Some(Keycode::Q),
                    keymod,
                    ..
                } if keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD) => break 'main,
                // Ctrl+S → soft screen-off (Ctrl combos don't generate text input)
                Event::KeyDown {
                    keycode: Some(Keycode::S),
                    keymod,
                    ..
                } if keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD) => {
                    toggle_soft_off(serial.as_deref(), &mut soft_off, &mut saved_display);
                }
                // nav / control keys → keyevent
                Event::KeyDown {
                    keycode: Some(k), ..
                } => {
                    if let Some(code) = keycode_to_android(k) {
                        keyevent(serial.as_deref(), code);
                    }
                }
                // printable text (incl. IME composition commit) → input text
                Event::TextInput { text, .. } if !text.is_empty() => {
                    text_input(serial.as_deref(), &text);
                }
                // left press → touch DOWN (drag start); tap = DOWN + UP
                Event::MouseButtonDown {
                    x,
                    y,
                    mouse_btn: MouseButton::Left,
                    ..
                } => {
                    if let Some((px, py)) = map_to_phone(x, y, win_w, win_h) {
                        drag = Some((px, py));
                        motion(serial.as_deref(), "DOWN", px, py);
                    }
                }
                // right click → back
                Event::MouseButtonDown {
                    x,
                    y,
                    mouse_btn: MouseButton::Right,
                    ..
                } => {
                    if map_to_phone(x, y, win_w, win_h).is_some() {
                        keyevent(serial.as_deref(), 4);
                    }
                }
                // middle click → home
                Event::MouseButtonDown {
                    x,
                    y,
                    mouse_btn: MouseButton::Middle,
                    ..
                } => {
                    if map_to_phone(x, y, win_w, win_h).is_some() {
                        keyevent(serial.as_deref(), 3);
                    }
                }
                // drag: track the pointer, stream MOVE events
                Event::MouseMotion { x, y, .. } => {
                    if drag.is_some() {
                        if let Some((px, py)) = map_to_phone(x, y, win_w, win_h) {
                            drag = Some((px, py));
                            motion(serial.as_deref(), "MOVE", px, py);
                        }
                    }
                }
                // left release → touch UP (tap or drag end)
                Event::MouseButtonUp {
                    x,
                    y,
                    mouse_btn: MouseButton::Left,
                    ..
                } => {
                    if let Some((sx, sy)) = drag.take() {
                        let pos = map_to_phone(x, y, win_w, win_h).unwrap_or((sx, sy));
                        motion(serial.as_deref(), "UP", pos.0, pos.1);
                    }
                }
                Event::MouseWheel { y, .. } if y != 0 => {
                    // wheel up (y>0) = finger drags down = scroll up; wheel down mirrors it
                    if let Some((px, py)) = map_to_phone(mouse.x(), mouse.y(), win_w, win_h) {
                        let d = 60 * y.signum();
                        let _ = shell(
                            serial.as_deref(),
                            &format!("input swipe {px} {} {px} {} 80", py - d, py + d),
                        );
                    }
                }
                _ => {}
            }
        }

        // drain whatever arrived from the reader thread
        let mut got_data = false;
        while let Ok(data) = rx.recv_timeout(Duration::from_millis(10)) {
            got_data = true;
            pending.extend_from_slice(&data);
            let starts = nal_starts(&pending);
            for w in starts.windows(2) {
                let nal = &pending[w[0]..w[1]];
                match decoder.decode(nal) {
                    Ok(Some(frame)) => {
                        let (w, h) = frame.dimensions();
                        let (sy, su, sv) = frame.strides();
                        let y = frame.y();
                        let u = frame.u();
                        let v = frame.v();
                        let ylen = w * h;
                        let uvlen = w * h / 4;
                        if tight.len() != ylen + uvlen * 2 {
                            tight.resize(ylen + uvlen * 2, 0);
                        }
                        for row in 0..h {
                            tight[row * w..(row + 1) * w]
                                .copy_from_slice(&y[row * sy..row * sy + w]);
                        }
                        for row in 0..h / 2 {
                            tight[ylen + row * (w / 2)..ylen + (row + 1) * (w / 2)]
                                .copy_from_slice(&u[row * su..row * su + w / 2]);
                            tight[ylen + uvlen + row * (w / 2)..ylen + uvlen + (row + 1) * (w / 2)]
                                .copy_from_slice(&v[row * sv..row * sv + w / 2]);
                        }
                        texture.update_yuv(
                            None,
                            &tight[..ylen],
                            w,
                            &tight[ylen..ylen + uvlen],
                            w / 2,
                            &tight[ylen + uvlen..],
                            w / 2,
                        )?;
                        let (win_w, win_h) = canvas.output_size()?;
                        let (dst, _) = view_rect(win_w, win_h);
                        canvas.copy(&texture, None, Some(dst))?;
                        canvas.present();
                        frames += 1;
                        if title_t.elapsed().as_secs() >= 2 {
                            let fps = frames as f32 / started.elapsed().as_secs_f32();
                            let state = if soft_off { " [SOFT-OFF]" } else { "" };
                            let _ = canvas
                                .window_mut()
                                .set_title(&format!("laphone M0 — {fps:.1} fps{state}"));
                            title_t = Instant::now();
                        }
                    }
                    Ok(None) => {} // need more data
                    Err(e) => {
                        decode_errors += 1;
                        if decode_errors <= 5 {
                            eprintln!("decode error: {e}");
                        }
                    }
                }
            }
            if let Some(&keep) = starts.last() {
                pending.drain(..keep); // keep only the trailing partial NAL
            }
        }
        if got_data {
            last_data = Instant::now();
        } else if last_data.elapsed() > Duration::from_secs(5) {
            // stream stalled: the screen may have fallen asleep (screenrecord
            // needs the display on). Wake it; if adb itself is unreachable
            // (USB unplugged), give up.
            if shell(serial.as_deref(), "input keyevent KEYCODE_WAKEUP").is_none() {
                eprintln!("adb unreachable — exiting");
                break 'main;
            }
            last_data = Instant::now();
        }
    }

    // restore display state if soft-off is still active
    if soft_off {
        toggle_soft_off(serial.as_deref(), &mut soft_off, &mut saved_display);
    }

    Ok(())
}
