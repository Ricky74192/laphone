// laphone-input — on-phone input server (native ARM64, runs from
// /data/local/tmp via adb shell). Creates a VIRTUAL TOUCHSCREEN through
// /dev/uinput (shell uid is in the uhid group) and injects kernel-level
// touch events — sub-ms per event, no hidden APIs, no root.
//
// Transport: abstract Unix socket "laphone_input" (adb forward
// tcp:27047 localabstract:laphone_input). Protocol (UTF-8 lines):
//   screen W H | tap x y | down x y | move x y | up x y | swipe x1 y1 x2 y2 ms
// The server keeps running across client disconnects; the uinput device
// lives until the process exits (kernel removes it when the fd closes).
#![cfg(target_arch = "aarch64")]

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader};
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::time::Duration;

const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const EV_SYN: u16 = 0x00;
const BTN_TOUCH: u16 = 0x14a;
const BTN_LEFT: u16 = 0x110;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const SYN_REPORT: u16 = 0x00;

const UI_SET_EVBIT: libc::c_int = 0x40045564;
const UI_SET_KEYBIT: libc::c_int = 0x40045565;
const UI_SET_ABSBIT: libc::c_int = 0x40045567;
const UI_DEV_CREATE: libc::c_int = 0x5501;
const UI_DEV_DESTROY: libc::c_int = 0x5502;

const DEV_MAX_X: i32 = 10799; // 10 units per display pixel (like the real fts touchscreen)
const DEV_MAX_Y: i32 = 23999;

#[repr(C)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
struct UinputUserDev {
    name: [u8; 80],
    id: InputId,
    ff_effects_max: u32,
    absmax: [i32; 64],
    absmin: [i32; 64],
    absfuzz: [i32; 64],
    absflat: [i32; 64],
}

#[repr(C)]
struct InputEvent {
    time: [libc::c_long; 2], // struct timeval
    ev_type: u16,
    code: u16,
    value: i32,
}

const EV_SIZE: usize = std::mem::size_of::<InputEvent>();

struct TouchScreen {
    fd: std::fs::File,
    w: f32,
    h: f32,
}

impl TouchScreen {
    fn create() -> std::io::Result<TouchScreen> {
        let f = OpenOptions::new().read(true).write(true).open("/dev/uinput")?;
        let fd = f.as_raw_fd();
        // classic API: UI_SET_* ioctls + write(struct uinput_user_dev) + UI_DEV_CREATE
        let mut dev: UinputUserDev = unsafe { std::mem::zeroed() };
        dev.name = {
            let mut n = [0u8; 80];
            let src = b"laphone-touch";
            n[..src.len()].copy_from_slice(src);
            n
        };
        dev.id.bustype = 0x01; // BUS_USB
        dev.id.vendor = 0x4c50;
        dev.id.product = 0x5443;
        dev.absmax[ABS_X as usize] = DEV_MAX_X;
        dev.absmax[ABS_Y as usize] = DEV_MAX_Y;
        dev.absmax[ABS_MT_SLOT as usize] = 9;
        dev.absmax[ABS_MT_TRACKING_ID as usize] = 0xffff;
        dev.absmax[ABS_MT_POSITION_X as usize] = DEV_MAX_X;
        dev.absmax[ABS_MT_POSITION_Y as usize] = DEV_MAX_Y;
        for (req, arg) in [
            (UI_SET_EVBIT, EV_KEY as libc::c_int),
            (UI_SET_EVBIT, EV_ABS as libc::c_int),
            (UI_SET_KEYBIT, BTN_TOUCH as libc::c_int),
            (UI_SET_KEYBIT, BTN_LEFT as libc::c_int),
            (UI_SET_ABSBIT, ABS_X as libc::c_int),
            (UI_SET_ABSBIT, ABS_Y as libc::c_int),
            (UI_SET_ABSBIT, ABS_MT_SLOT as libc::c_int),
            (UI_SET_ABSBIT, ABS_MT_TRACKING_ID as libc::c_int),
            (UI_SET_ABSBIT, ABS_MT_POSITION_X as libc::c_int),
            (UI_SET_ABSBIT, ABS_MT_POSITION_Y as libc::c_int),
        ] {
            if unsafe { libc::ioctl(fd, req, arg) } != 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("ioctl 0x{req:x} arg={arg}: {}", std::io::Error::last_os_error()),
                ));
            }
        }
        let wr = unsafe {
            libc::write(
                fd,
                &dev as *const _ as *const libc::c_void,
                std::mem::size_of::<UinputUserDev>(),
            )
        };
        if wr != std::mem::size_of::<UinputUserDev>() as isize {
            return Err(std::io::Error::last_os_error());
        }
        if unsafe { libc::ioctl(fd, UI_DEV_CREATE) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        println!("uinput created via classic API (ev_size={})", EV_SIZE);
        Ok(TouchScreen { fd: f, w: 1080.0, h: 2400.0 })
    }

    fn destroy(&mut self) {
        unsafe {
            libc::ioctl(self.fd.as_raw_fd(), UI_DEV_DESTROY);
        }
    }

    fn emit(&self, ev_type: u16, code: u16, value: i32) {
        let ev = InputEvent {
            time: [0, 0],
            ev_type,
            code,
            value,
        };
        let r = unsafe {
            libc::write(self.fd.as_raw_fd(), &ev as *const _ as *const libc::c_void, EV_SIZE)
        };
        if r != EV_SIZE as isize {
            println!(
                "emit failed: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    fn sync(&self) {
        self.emit(EV_SYN, SYN_REPORT, 0);
    }

    fn touch(&self, down: bool, x: f32, y: f32) {
        // device units are 10 per display pixel, matching the real fts touchscreen
        let dx = (x * 10.0) as i32;
        let dy = (y * 10.0) as i32;
        let dx = dx.clamp(0, DEV_MAX_X);
        let dy = dy.clamp(0, DEV_MAX_Y);
        // always update BOTH legacy axes and MT axes: the POINTER-mode
        // InputReader consumes ABS_X/ABS_Y while TOUCHSCREEN-mode
        // consumers use the MT axes
        self.emit(EV_ABS, ABS_X, dx);
        self.emit(EV_ABS, ABS_Y, dy);
        // MT protocol B: slot 0, tracking id (>=0 down, -1 up)
        self.emit(EV_ABS, ABS_MT_SLOT, 0);
        if down {
            self.emit(EV_ABS, ABS_MT_TRACKING_ID, 1);
            self.emit(EV_ABS, ABS_MT_POSITION_X, dx);
            self.emit(EV_ABS, ABS_MT_POSITION_Y, dy);
        } else {
            self.emit(EV_ABS, ABS_MT_TRACKING_ID, -1);
        }
        self.emit(EV_KEY, BTN_TOUCH, down as i32);
        self.emit(EV_KEY, BTN_LEFT, down as i32);
        self.sync();
    }

    /// Hover: position the pointer WITHOUT pressing any button (POINTER-mode
    /// InputReader may require an established pointer position before a
    /// click is accepted).
    fn hover(&self, x: f32, y: f32) {
        let dx = (x * 10.0) as i32;
        let dy = (y * 10.0) as i32;
        let dx = dx.clamp(0, DEV_MAX_X);
        let dy = dy.clamp(0, DEV_MAX_Y);
        self.emit(EV_ABS, ABS_X, dx);
        self.emit(EV_ABS, ABS_Y, dy);
        self.emit(EV_ABS, ABS_MT_SLOT, 0);
        self.emit(EV_ABS, ABS_MT_POSITION_X, dx);
        self.emit(EV_ABS, ABS_MT_POSITION_Y, dy);
        self.sync();
    }

    fn tap(&self, x: f32, y: f32) {
        self.touch(true, x, y);
        self.touch(false, x, y);
    }

    fn swipe(&self, x1: f32, y1: f32, x2: f32, y2: f32, dur_ms: u32) {
        let steps = (dur_ms / 16).max(1);
        self.touch(true, x1, y1);
        for i in 1..=steps {
            std::thread::sleep(Duration::from_millis(16));
            let t = i as f32 / steps as f32;
            self.touch(true, x1 + (x2 - x1) * t, y1 + (y2 - y1) * t);
        }
        self.touch(false, x2, y2);
    }
}

/// Abstract Unix socket server ("laphone_input"): returns the listening fd.
fn bind_abstract(name: &str) -> std::io::Result<i32> {
    unsafe {
        let fd = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut addr: libc::sockaddr_un = std::mem::zeroed();
        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
        let bytes = name.as_bytes();
        assert!(1 + bytes.len() <= addr.sun_path.len());
        addr.sun_path[0] = 0; // abstract namespace
        addr.sun_path[1..=bytes.len()].copy_from_slice(bytes);
        let addrlen = (std::mem::offset_of!(libc::sockaddr_un, sun_path) + 1 + bytes.len()) as libc::socklen_t;
        if libc::bind(fd, &addr as *const _ as *const libc::sockaddr, addrlen) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::listen(fd, 2) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(fd)
    }
}

fn main() {
    println!("laphone-input: creating uinput touchscreen");
    let mut ts = match TouchScreen::create() {
        Ok(ts) => ts,
        Err(e) => {
            println!("uinput create failed: {e}");
            std::process::exit(1);
        }
    };
    println!("uinput touchscreen ready");

    let listener = match bind_abstract("laphone_input") {
        Ok(fd) => fd,
        Err(e) => {
            println!("socket bind failed: {e}");
            ts.destroy();
            std::process::exit(1);
        }
    };
    println!("listening on laphone_input");

    loop {
        let fd = unsafe { libc::accept(listener, std::ptr::null_mut(), std::ptr::null_mut()) };
        if fd < 0 {
            println!("accept failed: {}", std::io::Error::last_os_error());
            break;
        }
        let stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let cmd = line.trim();
                    let parts: Vec<&str> = cmd.split(' ').collect();
                    match parts.first() {
                        Some(&"screen") if parts.len() >= 3 => {
                            ts.w = parts[1].parse().unwrap_or(ts.w);
                            ts.h = parts[2].parse().unwrap_or(ts.h);
                        }
                        Some(&"tap") if parts.len() >= 3 => {
                            let x: f32 = parts[1].parse().unwrap_or(0.0);
                            let y: f32 = parts[2].parse().unwrap_or(0.0);
                            ts.hover(x, y);
                            std::thread::sleep(Duration::from_millis(30));
                            ts.tap(x, y);
                        }
                        Some(&"down") if parts.len() >= 3 => {
                            let x: f32 = parts[1].parse().unwrap_or(0.0);
                            let y: f32 = parts[2].parse().unwrap_or(0.0);
                            ts.touch(true, x, y);
                        }
                        Some(&"move") if parts.len() >= 3 => {
                            let x: f32 = parts[1].parse().unwrap_or(0.0);
                            let y: f32 = parts[2].parse().unwrap_or(0.0);
                            ts.touch(true, x, y);
                        }
                        Some(&"up") if parts.len() >= 3 => {
                            let x: f32 = parts[1].parse().unwrap_or(0.0);
                            let y: f32 = parts[2].parse().unwrap_or(0.0);
                            ts.touch(false, x, y);
                        }
                        Some(&"swipe") if parts.len() >= 6 => {
                            let x1: f32 = parts[1].parse().unwrap_or(0.0);
                            let y1: f32 = parts[2].parse().unwrap_or(0.0);
                            let x2: f32 = parts[3].parse().unwrap_or(0.0);
                            let y2: f32 = parts[4].parse().unwrap_or(0.0);
                            let dur: u32 = parts[5].parse().unwrap_or(120);
                            ts.swipe(x1, y1, x2, y2, dur);
                        }
                        _ => {}
                    }
                }
            }
        }
        println!("client disconnected — waiting for next connection");
        // keep the uinput device; continue accepting
    }
    ts.destroy();
    println!("laphone-input: bye");
}
