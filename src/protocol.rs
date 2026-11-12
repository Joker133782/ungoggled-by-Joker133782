//! DJI logiclink framing and the small mobile video session.
//! Protocol references and provenance are recorded in FINDINGS.MD.
pub const VIDEO: u16 = 0x574a;
pub const CONTROL: u16 = 0x7530;
const MAX_PAYLOAD: usize = 2_000_000;

#[derive(Default)]
pub struct Framer {
    buf: Vec<u8>,
    pub discarded: u64,
}

impl Framer {
    pub fn feed(&mut self, data: &[u8]) -> Vec<(u16, Vec<u8>)> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        let mut pos = 0;
        while self.buf.len() - pos >= 8 {
            if self.buf[pos..pos + 2] != [0x55, 0xcc] {
                pos += 1;
                self.discarded += 1;
                continue;
            }
            let port = u16::from_le_bytes(self.buf[pos + 2..pos + 4].try_into().unwrap());
            let len = u32::from_le_bytes(self.buf[pos + 4..pos + 8].try_into().unwrap()) as usize;
            if len == 0 || len > MAX_PAYLOAD {
                pos += 1;
                self.discarded += 1;
                continue;
            }
            if self.buf.len() - pos < 8 + len {
                break;
            }
            out.push((port, self.buf[pos + 8..pos + 8 + len].to_vec()));
            pos += 8 + len;
        }
        self.buf.drain(..pos);
        out
    }
}

pub fn wrap(port: u16, data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x55, 0xcc];
    out.extend(port.to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(data);
    out
}

pub fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0x77;
    for b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0x8c } else { 0 };
        }
    }
    crc
}
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0x3692;
    for b in data {
        crc ^= *b as u16;
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0x8408 } else { 0 };
        }
    }
    crc
}

pub fn duml(
    sender: u8,
    receiver: u8,
    seq: u16,
    flags: u8,
    set: u8,
    id: u8,
    payload: &[u8],
) -> Vec<u8> {
    assert!(payload.len() <= 1010);
    let len = (payload.len() as u16 + 13) | 0x400;
    let mut p = vec![0x55, len as u8, (len >> 8) as u8];
    p.push(crc8(&p));
    p.extend([
        sender,
        receiver,
        seq as u8,
        (seq >> 8) as u8,
        flags,
        set,
        id,
    ]);
    p.extend(payload);
    p.extend(crc16(&p).to_le_bytes());
    p
}

pub fn valid(p: &[u8]) -> bool {
    p.len() >= 13
        && p[0] == 0x55
        && (u16::from_le_bytes([p[1], p[2]]) & 0x3ff) as usize == p.len()
        && crc8(&p[..3]) == p[3]
        && crc16(&p[..p.len() - 2]) == u16::from_le_bytes(p[p.len() - 2..].try_into().unwrap())
}

pub fn identity_reply(p: &[u8]) -> Option<Vec<u8>> {
    if !valid(p) || p[8] & 0x80 != 0 || p[9] != 0 || p[5] != 2 {
        return None;
    }
    let payload = match p[10] {
        0x81 => {
            let mut v = vec![0; 64];
            v[1..4].copy_from_slice(b"APP");
            v[32] = 2;
            v[44..46].copy_from_slice(&[5, 0x1c]);
            v
        }
        0x82 => vec![0],
        0x88 if p.get(11..13) == Some(&[0x19, 0]) => vec![0x1a, 0, 0, 0, 0],
        _ => return None,
    };
    Some(duml(
        p[5],
        p[4],
        u16::from_le_bytes([p[6], p[7]]),
        0x80,
        0,
        p[10],
        &payload,
    ))
}

pub fn unhex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
        .collect()
}

/// Two minimal mobile registrations reported working on N3/O4 Pro.
/// Fresh sequence numbers and CRCs; channel is selected for the goggles profile.
pub fn registration(seq: &mut u16, port: u16) -> Vec<Vec<u8>> {
    [
        (
            0x28,
            0x99,
            "02020000d507000000000013000d0063616d6361705f636f6d6d6f6e00000000",
        ),
        (0x3c, 0x88, "1700002300415050000000000002"),
    ]
    .iter()
    .map(|(receiver, id, payload)| {
        *seq = seq.wrapping_add(1);
        wrap(
            port,
            &duml(2, *receiver, *seq, 0x40, 0, *id, &unhex(payload)),
        )
    })
    .collect()
}

const TIME_RECEIVER: u8 = 0x28;
pub(crate) const TIME_SET_CALENDAR: u8 = 0x4a;

/// SetDateTime on the air-unit RTC. Verified on Goggles 3 with O3/O4: ACK 00.
pub fn time_sync(seq: &mut u16, port: u16, clock: &crate::clock::Wall) -> Vec<u8> {
    *seq = seq.wrapping_add(1);
    let mut payload = clock.year.to_le_bytes().to_vec();
    payload.extend([
        clock.month,
        clock.day,
        clock.hour,
        clock.minute,
        clock.second,
    ]);
    wrap(
        port,
        &duml(2, TIME_RECEIVER, *seq, 0x40, 0, TIME_SET_CALENDAR, &payload),
    )
}

pub fn is_time_sync(p: &[u8]) -> bool {
    valid(p) && p[9] == 0 && p[10] == TIME_SET_CALENDAR
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captured_crc_vectors() {
        let p = unhex("551b0475023cf4fe400088170000230041505000000000000258a6");
        assert!(valid(&p));
        assert_eq!(duml(2, 0x3c, 0xfef4, 0x40, 0, 0x88, &p[11..p.len() - 2]), p);
        let mut broken = p.clone();
        broken[15] ^= 1;
        assert!(!valid(&broken));
        let p = unhex(
            "552d04f20228f3fe40009902020000d507000000000013000d0063616d6361705f636f6d6d6f6e00000000d093",
        );
        assert!(valid(&p));
        let mut seq = 0xfef2;
        let packets = registration(&mut seq, 0x5749);
        assert_eq!(&packets[0][8..], p);
        assert_eq!(packets[1].len(), 35);
    }
    #[test]
    fn arbitrary_transfer_boundaries_and_multiple_frames() {
        let a = wrap(VIDEO, &[0, 0, 1, 0x67, 0x55, 0xcc]);
        let b = wrap(CONTROL, &[8; 19]);
        let stream = [a, b].concat();
        for split in 0..=stream.len() {
            let mut f = Framer::default();
            let mut got = f.feed(&stream[..split]);
            got.extend(f.feed(&stream[split..]));
            assert_eq!(
                got,
                vec![
                    (VIDEO, vec![0, 0, 1, 0x67, 0x55, 0xcc]),
                    (CONTROL, vec![8; 19])
                ]
            );
        }
    }
    #[test]
    fn corrupt_length_resynchronizes() {
        let mut f = Framer::default();
        let mut data = vec![0x55, 0xcc, 0x4a, 0x57, 0xff, 0xff, 0xff, 0xff];
        data.extend(wrap(VIDEO, &[1, 2, 3]));
        assert_eq!(f.feed(&data), vec![(VIDEO, vec![1, 2, 3])]);
        assert_eq!(f.discarded, 8);
    }
    #[test]
    fn response_preserves_address_and_sequence() {
        let req = duml(0x3c, 2, 123, 0x40, 0, 0x88, &[0x19, 0]);
        let response = identity_reply(&req).unwrap();
        assert!(valid(&response));
        assert_eq!(&response[4..11], &[2, 0x3c, 123, 0, 0x80, 0, 0x88]);
        assert!(identity_reply(&response).is_none());
    }
    #[test]
    fn time_sync_sends_calendar_set_to_rtc_endpoint() {
        let clock = crate::clock::Wall {
            unix_ms: 1_700_000_000_000,
            year: 2026,
            month: 9,
            day: 20,
            hour: 18,
            minute: 15,
            second: 7,
            tz: "Europe/Berlin".into(),
        };
        let mut seq = 0x10;
        let packet = time_sync(&mut seq, CONTROL, &clock);
        assert_eq!(seq, 0x11);
        let frame = &packet[8..];
        assert!(valid(frame));
        assert!(is_time_sync(frame));
        assert_eq!(&frame[4..6], &[2, TIME_RECEIVER]);
        assert_eq!(&frame[8..11], &[0x40, 0, TIME_SET_CALENDAR]);
        assert_eq!(&frame[11..frame.len() - 2], [0xea, 0x07, 9, 20, 18, 15, 7]);
        let ack = duml(TIME_RECEIVER, 2, 0x11, 0xc0, 0, TIME_SET_CALENDAR, &[0]);
        assert!(is_time_sync(&ack));
        assert!(!is_time_sync(&unhex(
            "551b0475023cf4fe400088170000230041505000000000000258a6"
        )));
    }
}
