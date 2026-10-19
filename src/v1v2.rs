//! Experimental USB-host receiver for the original DJI FPV Goggles (V1/V2).
//!
//! Goggles 2/3 make the Pi a USB *accessory* that the goggles, acting as USB
//! host, connect to (see `gadget.rs`/`functionfs.rs`). V1/V2 work the other
//! way around: the goggles expose a plain USB bulk interface and something
//! else has to be the USB host. That side was reverse-engineered by the
//! community project <https://github.com/fpv-wtf/voc-poc>: send the 4-byte
//! magic packet `0x524d5654` to the bulk OUT endpoint, then read raw bytes
//! from the bulk IN endpoint. The upstream project calls the exact framing of
//! those bytes "currently largely unknown" and only claims it "sort of just
//! works" once piped into ffmpeg, which finds Annex-B start codes on its own.
//! We rely on the same tolerance in `h264::AnnexB` downstream. Nothing in
//! this file has been run against real V1/V2 hardware.
use crate::{WorkerArgs, video};
use anyhow::{Context, Result};
use nusb::{
    MaybeFuture,
    descriptors::{InterfaceDescriptor, TransferType},
    transfer::{Bulk, Direction, In, Out},
};
use serde_json::json;
use std::{
    io::{Read, Write},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

/// Vendor and product ID voc-poc uses for both V1 and V2; DJI has not
/// published separate IDs for the two.
const VENDOR_ID: u16 = 0x2ca3;
const PRODUCT_ID: u16 = 0x1f;
/// The bulk interface carrying the video stream, per voc-poc.
const INTERFACE: u8 = 3;
const MAGIC: [u8; 4] = [0x52, 0x4d, 0x56, 0x54];

fn event(value: serde_json::Value) {
    println!("{value}");
}

/// Pick the first bulk OUT and bulk IN endpoint addresses on an interface.
/// Kept separate from the USB I/O so it can be tested without hardware.
fn bulk_endpoints(descriptor: &InterfaceDescriptor) -> Result<(u8, u8)> {
    let mut out_addr = None;
    let mut in_addr = None;
    for ep in descriptor.endpoints() {
        if ep.transfer_type() != TransferType::Bulk {
            continue;
        }
        match ep.direction() {
            Direction::Out if out_addr.is_none() => out_addr = Some(ep.address()),
            Direction::In if in_addr.is_none() => in_addr = Some(ep.address()),
            _ => {}
        }
    }
    Ok((
        out_addr.context("Goggles interface has no bulk OUT endpoint")?,
        in_addr.context("Goggles interface has no bulk IN endpoint")?,
    ))
}

pub fn run(args: &WorkerArgs) -> Result<()> {
    event(json!({"phase":"starting","video_bytes":0,"bitrate_mbps":0}));

    let device_info = nusb::list_devices()
        .wait()
        .context("Cannot enumerate USB devices")?
        .find(|d| d.vendor_id() == VENDOR_ID && d.product_id() == PRODUCT_ID)
        .context(
            "Goggles not found. V1/V2 need a USB-A host port (not the gadget USB-C port) \
             with a data-capable cable, and the goggles powered on with a stream connected.",
        )?;
    let device = device_info
        .open()
        .wait()
        .context("Cannot open goggles USB device")?;
    let interface = device
        .claim_interface(INTERFACE)
        .wait()
        .context("Cannot claim goggles bulk interface")?;
    let descriptor = interface
        .descriptor()
        .context("Goggles interface has no active alternate setting")?;
    let (out_addr, in_addr) = bulk_endpoints(&descriptor)?;

    event(json!({"phase":"usb_connected"}));

    let mut writer = interface
        .endpoint::<Bulk, Out>(out_addr)
        .context("Cannot open bulk OUT endpoint")?
        .writer(4096);
    writer
        .write_all(&MAGIC)
        .context("Cannot send magic packet")?;
    writer.flush().context("Cannot flush magic packet")?;

    let mut reader = interface
        .endpoint::<Bulk, In>(in_addr)
        .context("Cannot open bulk IN endpoint")?
        .reader(16384);

    event(json!({"phase":"waiting_video"}));

    let video = video::Output::connect(&args.video_socket);
    let total = Arc::new(AtomicU64::new(0));
    let reporter_total = total.clone();
    // Detached: this worker is a dedicated subprocess, so there is nothing to
    // join it with, and the thread exits when the process does.
    thread::spawn(move || {
        let mut last = 0;
        loop {
            thread::sleep(Duration::from_secs(1));
            let now = reporter_total.load(Ordering::Relaxed);
            event(json!({
                "video_bytes": now,
                "bitrate_mbps": (now - last) as f64 * 8.0 / 1_000_000.0,
                "phase": if now > last { "streaming" } else { "waiting_video" },
            }));
            last = now;
        }
    });
    let mut buffer = vec![0u8; 16384];
    loop {
        let n = match reader.read(&mut buffer) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
                continue;
            }
            result => result.context("Read goggles USB bulk IN")?,
        };
        if n == 0 {
            continue;
        }
        total.fetch_add(n as u64, Ordering::Relaxed);
        video.send(buffer[..n].to_vec());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nusb::descriptors::ConfigurationDescriptor;

    fn interface_descriptors(num_endpoints: u8, endpoints: &[u8]) -> Vec<u8> {
        let mut config = vec![9, 2, 0, 0, 1, 1, 0, 0xc0, 1];
        config.extend([9, 4, 3, 0, num_endpoints, 0xff, 0xff, 0, 0]);
        config.extend_from_slice(endpoints);
        let len = config.len() as u16;
        config[2] = len as u8;
        config[3] = (len >> 8) as u8;
        config
    }

    fn endpoint(address: u8, attributes: u8) -> [u8; 7] {
        [7, 5, address, attributes, 0, 2, 0]
    }

    #[test]
    fn picks_first_bulk_in_and_out() {
        let mut eps = endpoint(0x01, 2).to_vec(); // bulk out
        eps.extend(endpoint(0x81, 2)); // bulk in
        eps.extend(endpoint(0x82, 3)); // interrupt in, ignored
        let bytes = interface_descriptors(3, &eps);
        let config = ConfigurationDescriptor::new(&bytes).unwrap();
        let interface = config
            .interfaces()
            .find(|i| i.interface_number() == 3)
            .unwrap()
            .first_alt_setting();
        assert_eq!(bulk_endpoints(&interface).unwrap(), (0x01, 0x81));
    }

    #[test]
    fn rejects_interface_without_bulk_endpoints() {
        let eps = endpoint(0x82, 3); // interrupt in only
        let bytes = interface_descriptors(1, &eps);
        let config = ConfigurationDescriptor::new(&bytes).unwrap();
        let interface = config
            .interfaces()
            .find(|i| i.interface_number() == 3)
            .unwrap()
            .first_alt_setting();
        assert!(bulk_endpoints(&interface).is_err());
    }
}
