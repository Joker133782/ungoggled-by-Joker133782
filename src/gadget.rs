use crate::{WorkerArgs, protocol, session, video};
use anyhow::{Context, Result, bail};
use serde_json::json;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::fd::AsRawFd,
    path::Path,
    sync::{Arc, Mutex, atomic::Ordering},
    thread,
    time::{Duration, Instant},
};

fn event(value: serde_json::Value) {
    println!("{value}");
}

fn endpoint(addr: u8, packet: u16) -> Vec<u8> {
    vec![7, 5, addr, 2, packet as u8, (packet >> 8) as u8, 0]
}

pub fn descriptors(accessory: bool, accessory_pid: u16) -> Vec<u8> {
    let mut out = vec![0; 4]; // gadgetfs native-endian descriptor tag
    for packet in [64, 512] {
        out.extend([9, 2, 32, 0, 1, 1, 0, 0xc0, 1]); // self-powered
        out.extend([9, 4, 0, 0, 2, 0xff, 0xff, 0, 0]);
        out.extend(endpoint(1, packet));
        out.extend(endpoint(0x81, packet));
    }
    let pid: u16 = if accessory { accessory_pid } else { 0x4ee0 };
    out.extend([
        18,
        1,
        0,
        2,
        0,
        0,
        0,
        64,
        0xd1,
        0x18,
        pid as u8,
        (pid >> 8) as u8,
        0,
        1,
        1,
        2,
        3,
        1,
    ]);
    out
}

fn open_ep(dir: &Path, addr: u8) -> Result<File> {
    let name = if addr & 0x80 != 0 { "ep1in" } else { "ep1out" };
    let mut ep = OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.join(name))
        .with_context(|| format!("open {name}"))?;
    let mut desc = 1u32.to_ne_bytes().to_vec();
    desc.extend(endpoint(addr, 64));
    desc.extend(endpoint(addr, 512));
    ep.write_all(&desc)
        .with_context(|| format!("configure {name}"))?;
    Ok(ep)
}

fn control_read(ep: &File, data: &mut [u8]) -> std::io::Result<usize> {
    // read() with length zero is meaningful for gadgetfs (USB status ACK).
    // std::io helpers are allowed to optimize empty reads away.
    let n = unsafe { libc::read(ep.as_raw_fd(), data.as_mut_ptr().cast(), data.len()) };
    if n < 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EIDRM) {
            Ok(0)
        } else {
            Err(error)
        }
    } else {
        Ok(n as usize)
    }
}
fn control_write(ep: &File, data: &[u8]) -> std::io::Result<usize> {
    let n = unsafe { libc::write(ep.as_raw_fd(), data.as_ptr().cast(), data.len()) };
    if n < 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EIDRM) {
            Ok(0)
        } else {
            Err(error)
        }
    } else {
        Ok(n as usize)
    }
}

pub fn run(args: &WorkerArgs) -> Result<()> {
    let dir = Path::new(&args.gadget_dir);
    let controller = if let Some(c) = &args.controller {
        c.clone()
    } else {
        std::fs::read_dir("/sys/class/udc")?
            .next()
            .context("No USB device controller. Use Pi 4 USB-C with dwc2 peripheral mode.")??
            .file_name()
            .to_string_lossy()
            .into_owned()
    };
    let mut accessory = args.cold_accessory;
    loop {
        let mut ep0 = OpenOptions::new()
            .read(true)
            .write(true)
            .open(dir.join(&controller))
            .context("Open gadgetfs controller; mount gadgetfs and stop other USB gadget owners")?;
        ep0.write_all(&descriptors(accessory, args.accessory_pid))
            .context("Bind gadgetfs descriptors")?;
        event(
            json!({"phase":if accessory {"accessory"} else {"waiting_usb"}, "message":"Waiting for goggles USB host"}),
        );
        let mut configured = false;
        let mut speed = 0;
        loop {
            let mut raw = [0u8; 12];
            let n = control_read(&ep0, &mut raw)?;
            if n == 0 {
                continue;
            }
            if n != 12 {
                bail!("Unexpected gadgetfs event length: {n}");
            }
            let kind = u32::from_ne_bytes(raw[8..12].try_into().unwrap());
            match kind {
                1 => {
                    speed = u32::from_ne_bytes(raw[..4].try_into().unwrap());
                    event(
                        json!({"phase":"usb_connected","message":format!("USB speed code {speed}")}),
                    );
                }
                2 => bail!("Goggles disconnected"),
                4 => eprintln!("USB suspend"),
                3 => {
                    let rt = raw[0];
                    let request = raw[1];
                    let value = u16::from_le_bytes([raw[2], raw[3]]);
                    let index = u16::from_le_bytes([raw[4], raw[5]]);
                    let len = u16::from_le_bytes([raw[6], raw[7]]) as usize;
                    eprintln!(
                        "USB setup {rt:02x}:{request:02x} value={value:04x} index={index} length={len}"
                    );
                    match (rt, request) {
                        (0xc0, 51) if len == 2 => {
                            control_write(&ep0, &[2, 0])?;
                        }
                        (0x40, 52) if index < 6 && len <= 256 => {
                            let mut b = vec![0; len];
                            let n = control_read(&ep0, &mut b)?;
                            event(
                                json!({"phase":"aoa_negotiation","message":format!("AOA string {index}: {}",String::from_utf8_lossy(&b[..n]).trim_end_matches('\0'))}),
                            );
                        }
                        (0x40, 53) if len == 0 => {
                            control_read(&ep0, &mut [])?;
                            thread::sleep(Duration::from_millis(100));
                            accessory = true;
                            break;
                        }
                        (0x80, 6) if value >> 8 == 3 => {
                            let s = match value as u8 {
                                1 => Some("Android"),
                                2 => Some("ungoggled"),
                                3 => Some("dji-hdmi-002"),
                                _ => None,
                            };
                            let bytes = if value as u8 == 0 {
                                vec![4, 3, 9, 4]
                            } else if let Some(s) = s {
                                let utf16: Vec<_> =
                                    s.encode_utf16().flat_map(u16::to_le_bytes).collect();
                                [vec![(utf16.len() + 2) as u8, 3], utf16].concat()
                            } else {
                                let _ = control_read(&ep0, &mut []);
                                continue;
                            };
                            control_write(&ep0, &bytes[..len.min(bytes.len())])?;
                        }
                        (0, 9) if value == 1 => {
                            if accessory && !configured {
                                if speed != 3 {
                                    eprintln!(
                                        "Warning: USB is not high speed; live video may be limited"
                                    );
                                }
                                let input = open_ep(dir, 1)?;
                                let output = open_ep(dir, 0x81)?;
                                control_read(&ep0, &mut [])?;
                                start_bulk(input, output, args.clone());
                                configured = true;
                                event(
                                    json!({"phase":"waiting_video","message":"Accessory configured; requesting live video"}),
                                );
                            } else {
                                control_read(&ep0, &mut [])?;
                            }
                        }
                        (0, 9) if value == 0 => {
                            control_read(&ep0, &mut [])?;
                            if configured {
                                bail!("USB deconfigured");
                            }
                        }
                        (0x81, 10) => {
                            control_write(&ep0, &[0][..len.min(1)])?;
                        }
                        (1, 11) if value == 0 => {
                            control_read(&ep0, &mut [])?;
                        }
                        _ => {
                            // Opposite-direction operation stalls an unsupported request.
                            if rt & 0x80 != 0 {
                                let _ = control_read(&ep0, &mut []);
                            } else {
                                let _ = control_write(&ep0, &[]);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        drop(ep0);
        thread::sleep(Duration::from_millis(500));
    }
}

pub(crate) fn start_bulk(mut input: File, output: File, args: WorkerArgs) {
    let output = Arc::new(Mutex::new(output));
    let tx = output.clone();
    let shared = Arc::new(session::Shared::default());
    let driver_state = shared.clone();
    let status_state = shared.clone();
    let config = session::Config {
        mode: args.goggles.clone(),
        g3_port: args.control_port,
        sync_time: args.sync_time,
        memory: session::memory_path(&args.video_socket),
    };
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        let mut send = |packet: &[u8]| write_bulk(&mut *tx.lock().unwrap(), packet);
        let result = session::drive(
            &config,
            &driver_state,
            &session::Timing::default(),
            &mut send,
            &event,
        );
        if let Err(e) = result {
            eprintln!("USB TX: {e}");
            std::process::exit(1);
        }
    });
    thread::spawn(move || {
        let mut last = 0;
        loop {
            thread::sleep(Duration::from_secs(1));
            let now = status_state.video.load(Ordering::Relaxed);
            event(
                json!({"video_bytes":now,"bitrate_mbps":(now-last) as f64 * 8.0 / 1_000_000.0,"phase":if now > last {"streaming"} else {"waiting_video"}}),
            );
            last = now;
        }
    });
    thread::spawn(move || {
        let result = (|| -> Result<()> {
            let mut framer = protocol::Framer::default();
            let mut buffer = [0u8; 16384];
            let mut capture = args.capture.as_ref().map(File::create).transpose()?;
            let mut captured = 0u64;
            let video = video::Output::connect(&args.video_socket);
            let mut controls = 0u64;
            let mut clock_ack = false;
            let mut last_log = Instant::now();
            loop {
                let n = match input.read(&mut buffer) {
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    result => result.context("Read USB bulk OUT")?,
                };
                if n == 0 {
                    continue;
                }
                for (port, payload) in framer.feed(&buffer[..n]) {
                    if port == protocol::VIDEO {
                        shared
                            .video
                            .fetch_add(payload.len() as u64, Ordering::Relaxed);
                        if captured < args.capture_limit
                            && let Some(file) = capture.as_mut()
                        {
                            let n = payload.len().min((args.capture_limit - captured) as usize);
                            file.write_all(&payload[..n])?;
                            captured += n as u64;
                        }
                        video.send(payload);
                    } else if port == protocol::CONTROL || port == 0x5749 {
                        controls += 1;
                        if protocol::valid(&payload) {
                            if protocol::is_time_sync(&payload) {
                                let body = &payload[11..payload.len() - 2];
                                if body == [0] {
                                    if !clock_ack {
                                        eprintln!("air unit clock set");
                                        clock_ack = true;
                                    }
                                } else {
                                    eprintln!("air unit clock rejected payload={body:02x?}");
                                }
                            } else if controls < 30 {
                                eprintln!(
                                    "DUML {:02x}->{:02x} {:02x}:{:02x} flags={:02x} payload={:02x?}",
                                    payload[4],
                                    payload[5],
                                    payload[9],
                                    payload[10],
                                    payload[8],
                                    &payload[11..payload.len() - 2]
                                );
                            }
                            if let Some(reply) = protocol::identity_reply(&payload) {
                                // Goggles 2 expect replies on 0x7530; Goggles 3 on
                                // the configured mobile control port.
                                let port = shared.reply_port(args.control_port);
                                write_bulk(
                                    &mut *output.lock().unwrap(),
                                    &protocol::wrap(port, &reply),
                                )?;
                            }
                        }
                    }
                }
                if last_log.elapsed() >= Duration::from_secs(2) {
                    event(json!({"control_packets":controls,"discarded_bytes":framer.discarded}));
                    last_log = Instant::now();
                }
            }
        })();
        if let Err(e) = result {
            eprintln!("Bulk receiver: {e:#}");
            std::process::exit(1);
        }
    });
}

// FunctionFS can transiently return EAGAIN even on a blocking endpoint. Do not
// tear down a working accessory session on the first temporary backpressure.
fn write_bulk(writer: &mut impl Write, mut data: &[u8]) -> std::io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !data.is_empty() {
        match writer.write(data) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(n) => data = &data[n..],
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(2));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn transient_usb_backpressure_preserves_all_bytes() {
        struct Endpoint {
            calls: usize,
            sent: Vec<u8>,
        }
        impl std::io::Write for Endpoint {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                self.calls += 1;
                match self.calls {
                    1 => Err(std::io::ErrorKind::Interrupted.into()),
                    3 => Err(std::io::ErrorKind::WouldBlock.into()),
                    _ => {
                        self.sent.push(data[0]);
                        Ok(1)
                    }
                }
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut endpoint = Endpoint {
            calls: 0,
            sent: vec![],
        };
        super::write_bulk(&mut endpoint, b"registration").unwrap();
        assert_eq!(endpoint.sent, b"registration");
    }
    #[test]
    fn descriptors_use_legal_speed_packet_sizes() {
        let d = super::descriptors(true, 0x2d00);
        assert_eq!(d.len(), 86);
        assert_eq!(&d[26..28], &[64, 0]);
        assert_eq!(&d[58..60], &[0, 2]);
        assert_eq!(&d[76..80], &[0xd1, 0x18, 0, 0x2d]);
    }
}
