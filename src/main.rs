// laphone M0 PoC — zero-install pipeline:
//   adb exec-out screenrecord --output-format=h264 → openh264 decode → SDL2 window
//   left click → adb shell input tap ; ESC / window close → quit
//
// Design notes:
// - Reader thread pushes raw stream chunks over an mpsc channel so the SDL
//   event loop never blocks on the adb pipe.
// - The reader thread restarts the recorder whenever the stream ends (device
//   sleep, disconnect, or a future ROM's recording cap); the decoder resyncs
//   on the fresh SPS/PPS of the new stream.
// - OpenH264 strides are padded (e.g. 1080 → 1088), but SDL IYUV textures need
//   exact pitches, so each frame is repacked into tight Y/U/V planes.
use std::env;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use openh264::decoder::Decoder;
use openh264::formats::YUVSource;
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::mouse::MouseButton;
use sdl2::pixels::PixelFormatEnum;

const PHONE_W: u32 = 1080; // TODO: auto-detect via `adb shell wm size`
const PHONE_H: u32 = 2400;
const SCALE: u32 = 2; // window = phone / SCALE
const BITRATE: &str = "8M";

fn spawn_recorder(serial: Option<&str>) -> std::io::Result<Child> {
    let mut cmd = Command::new("adb");
    if let Some(s) = serial {
        cmd.arg("-s").arg(s);
    }
    cmd.arg("exec-out")
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

fn tap(serial: Option<&str>, x: i32, y: i32) {
    let mut cmd = Command::new("adb");
    if let Some(s) = serial {
        cmd.arg("-s").arg(s);
    }
    let _ = cmd.arg("shell").arg(format!("input tap {x} {y}")).status();
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
    let serial = env::args().nth(1); // optional: adb serial

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
    let window = video
        .window("laphone M0", PHONE_W / SCALE, PHONE_H / SCALE)
        .position_centered()
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

    'main: loop {
        for ev in events.poll_iter() {
            match ev {
                Event::Quit { .. } => break 'main,
                Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => break 'main,
                Event::MouseButtonDown {
                    x,
                    y,
                    mouse_btn: MouseButton::Left,
                    ..
                } => tap(serial.as_deref(), x * SCALE as i32, y * SCALE as i32),
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
                        canvas.copy(&texture, None, None)?;
                        canvas.present();
                        frames += 1;
                        if title_t.elapsed().as_secs() >= 2 {
                            let fps = frames as f32 / started.elapsed().as_secs_f32();
                            let _ = canvas
                                .window_mut()
                                .set_title(&format!("laphone M0 — {fps:.1} fps"));
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
            eprintln!("stream stalled (adb died?) — exiting");
            break 'main;
        }
    }

    Ok(())
}
