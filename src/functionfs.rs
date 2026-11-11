//! Configfs + FunctionFS USB device transport. Owns only its named gadget.
use crate::{WorkerArgs, gadget};
use anyhow::{Context, Result, bail};
use serde_json::json;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{fd::AsRawFd, unix::fs::symlink},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};
const CONFIG: &str = "/sys/kernel/config/usb_gadget/dji_hdmi";
const MOUNT: &str = "/dev/dji-hdmi";
// How long the controller may report a configured host without the function
// producing a single event before the binding is cycled, and how often.
const SILENT_HOST: Duration = Duration::from_secs(10);
const SILENT_HOST_RECOVERIES: u32 = 3;

pub fn first_controller() -> Option<String> {
    fs::read_dir("/sys/class/udc")
        .ok()?
        .flatten()
        .next()
        .map(|e| e.file_name().to_string_lossy().into_owned())
}
pub fn udc_state(controller: &str) -> Option<String> {
    fs::read_to_string(format!("/sys/class/udc/{controller}/state"))
        .ok()
        .map(|s| s.trim().to_owned())
}

fn endpoint(addr: u8, packet: u16) -> Vec<u8> {
    vec![7, 5, addr, 2, packet as u8, (packet >> 8) as u8, 0]
}
pub fn descriptors() -> Vec<u8> {
    let mut out = Vec::new();
    for word in [3u32, 66, 1 | 2 | 64 | 128, 3, 3] {
        out.extend(word.to_le_bytes());
    }
    for size in [64, 512] {
        out.extend([9, 4, 0, 0, 2, 0xff, 0xff, 0, 1]);
        out.extend(endpoint(1, size));
        out.extend(endpoint(0x81, size));
    }
    out
}
fn strings() -> Vec<u8> {
    let s = b"ungoggled\0";
    let mut out = Vec::new();
    for word in [2u32, (18 + s.len()) as u32, 1, 1] {
        out.extend(word.to_le_bytes());
    }
    out.extend([9, 4]);
    out.extend(s);
    out
}
fn write_attr(path: impl AsRef<Path>, value: impl AsRef<[u8]>) -> Result<()> {
    fs::write(path.as_ref(), value).with_context(|| format!("write {}", path.as_ref().display()))
}
struct Binding;
impl Drop for Binding {
    fn drop(&mut self) {
        let _ = fs::write(format!("{CONFIG}/UDC"), "\n");
    }
}

// An update stops the service, swaps the release and starts it again seconds
// later. The goggles can come back with the port still configured while never
// re-running the accessory handshake, leaving the gadget bound and silent. Cycle
// the binding so the host re-enumerates, which is what a manual restart does.
// Silence with no host attached is normal and never triggers this.
fn recover_silent_host(config: PathBuf, controller: String, events: Arc<AtomicU64>) {
    thread::spawn(move || {
        for _ in 0..SILENT_HOST_RECOVERIES {
            loop {
                let seen = events.load(Ordering::Relaxed);
                thread::sleep(SILENT_HOST);
                if events.load(Ordering::Relaxed) != seen {
                    return; // The function is being driven; nothing to repair.
                }
                if udc_state(&controller).as_deref() == Some("configured") {
                    break;
                }
            }
            eprintln!("Host configured but FunctionFS silent; re-enumerating gadget");
            let _ = fs::write(config.join("UDC"), "\n");
            thread::sleep(Duration::from_millis(500));
            let _ = fs::write(config.join("UDC"), controller.as_bytes());
        }
    });
}

pub fn run(args: &WorkerArgs) -> Result<()> {
    let config = Path::new(CONFIG);
    if !Path::new("/sys/kernel/config/usb_gadget").exists() {
        bail!("configfs USB gadget support missing; run modprobe libcomposite");
    }
    fs::create_dir_all(config)?;
    let _ = fs::write(config.join("UDC"), "\n");
    let binding = Binding;
    let controller = args
        .controller
        .clone()
        .or_else(first_controller)
        .context("No USB device controller")?;
    for (key, value) in [
        ("idVendor", "0x18d1"),
        (
            "idProduct",
            if args.cold_accessory {
                if args.accessory_pid == 11521 {
                    "0x2d01"
                } else {
                    "0x2d00"
                }
            } else {
                "0x4ee0"
            },
        ),
        ("bcdUSB", "0x0200"),
        ("bcdDevice", "0x0100"),
        ("bDeviceClass", "0"),
        ("bDeviceSubClass", "0"),
        ("bDeviceProtocol", "0"),
    ] {
        write_attr(config.join(key), value)?;
    }
    fs::create_dir_all(config.join("strings/0x409"))?;
    for (key, value) in [
        ("manufacturer", "Android"),
        ("product", "ungoggled"),
        ("serialnumber", "dji-hdmi-ffs-001"),
    ] {
        write_attr(config.join("strings/0x409").join(key), value)?;
    }
    fs::create_dir_all(config.join("configs/c.1/strings/0x409"))?;
    write_attr(config.join("configs/c.1/MaxPower"), "2")?;
    write_attr(config.join("configs/c.1/bmAttributes"), "0xc0")?;
    fs::create_dir_all(config.join("functions/ffs.dji"))?;
    let link = config.join("configs/c.1/ffs.dji");
    if !link.exists() {
        symlink(config.join("functions/ffs.dji"), link)?;
    }
    fs::create_dir_all(MOUNT)?;
    if !fs::read_to_string("/proc/mounts")?
        .lines()
        .any(|l| l.split_whitespace().nth(1) == Some(MOUNT))
    {
        let status = Command::new("mount")
            .args(["-t", "functionfs", "dji", MOUNT])
            .status()?;
        if !status.success() {
            bail!("Cannot mount FunctionFS");
        }
    }
    let mut ep0 = OpenOptions::new()
        .read(true)
        .write(true)
        .open(format!("{MOUNT}/ep0"))?;
    ep0.write_all(&descriptors())
        .context("FunctionFS descriptors")?;
    ep0.write_all(&strings()).context("FunctionFS strings")?;
    // Endpoints exist only after descriptors have been supplied.
    let input = OpenOptions::new()
        .read(true)
        .write(true)
        .open(format!("{MOUNT}/ep1"))?;
    let output = OpenOptions::new()
        .read(true)
        .write(true)
        .open(format!("{MOUNT}/ep2"))?;
    let mut endpoints = Some((input, output));
    write_attr(config.join("UDC"), controller.as_bytes())?;
    println!(
        "{}",
        json!({"phase":"waiting_usb","message":"FunctionFS ready for goggles"})
    );
    let events = Arc::new(AtomicU64::new(0));
    recover_silent_host(config.to_path_buf(), controller.clone(), events.clone());
    let mut accessory = args.cold_accessory;
    let mut aoa = vec![String::new(); 6];
    let mut transitioning = false;
    let mut started = false;
    loop {
        let mut e = [0u8; 12];
        ep0.read_exact(&mut e).context("FunctionFS event")?;
        events.fetch_add(1, Ordering::Relaxed);
        let kind = e[8];
        match kind {
            0 | 1 => {} // bind/unbind occur during AOA re-enumeration
            2 => {
                println!(
                    "{}",
                    json!({"phase":if accessory {"waiting_video"}else{"usb_connected"},"message":"FunctionFS enabled"})
                );
                if accessory && !started {
                    let (input, output) = endpoints.take().unwrap();
                    gadget::start_bulk(input, output, args.clone(), aoa.clone());
                    started = true;
                }
                transitioning = false;
            }
            3 if started && !transitioning => bail!("Goggles disconnected or disabled USB"),
            3 => eprintln!("FunctionFS disabled (bus reset or re-enumeration)"),
            5 => eprintln!("USB suspend"),
            6 => eprintln!("USB resume"),
            4 => {
                let rt = e[0];
                let req = e[1];
                let len = u16::from_le_bytes([e[6], e[7]]) as usize;
                let idx = u16::from_le_bytes([e[4], e[5]]);
                eprintln!(
                    "FFS setup {:02x}:{:02x} index={} length={}",
                    rt, req, idx, len
                );
                match (rt, req) {
                    (0xc0, 51) if len == 2 => {
                        io(&ep0, true, &mut [2, 0])?;
                    }
                    (0x40, 52) if len <= 256 && idx < 6 => {
                        let mut s = vec![0; len];
                        let n = io(&ep0, false, &mut s)?;
                        let text = String::from_utf8_lossy(&s[..n])
                            .trim_end_matches('\0')
                            .to_owned();
                        eprintln!("AOA string {idx}: {text}");
                        aoa[idx as usize] = text;
                    }
                    (0x40, 53) if len == 0 => {
                        io(&ep0, false, &mut [])?;
                        thread::sleep(Duration::from_millis(100));
                        transitioning = true;
                        write_attr(config.join("UDC"), "\n")?;
                        thread::sleep(Duration::from_millis(500));
                        write_attr(
                            config.join("idProduct"),
                            format!("0x{:04x}", args.accessory_pid),
                        )?;
                        accessory = true;
                        write_attr(config.join("UDC"), controller.as_bytes())?;
                        println!(
                            "{}",
                            json!({"phase":"accessory","message":"AOA start accepted; re-enumerating"})
                        );
                    }
                    _ => {
                        let _ = io(&ep0, rt & 0x80 == 0, &mut []);
                    }
                }
            }
            _ => {}
        }
        // Keep the unbinding guard alive until all I/O has finished.
        let _ = &binding;
    }
}

fn io(ep: &File, write: bool, data: &mut [u8]) -> Result<usize> {
    let n = unsafe {
        if write {
            libc::write(ep.as_raw_fd(), data.as_ptr().cast(), data.len())
        } else {
            libc::read(ep.as_raw_fd(), data.as_mut_ptr().cast(), data.len())
        }
    };
    if n < 0 {
        Err(std::io::Error::last_os_error().into())
    } else {
        Ok(n as usize)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn functionfs_lengths() {
        let b = super::descriptors();
        assert_eq!(b.len(), 66);
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 66);
        let s = super::strings();
        assert_eq!(
            u32::from_le_bytes(s[4..8].try_into().unwrap()) as usize,
            s.len()
        );
    }
}
