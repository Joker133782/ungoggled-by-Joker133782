//! Goggles 2 (O3 generation) session: the arming burst and keepalives DJI Fly
//! sends, on the logiclink control port 0x7530. See `g2_sequence.rs` for the
//! provenance of the recorded requests.
use crate::{
    clock::Wall,
    g2_sequence::{ARM, SUSTAIN, Step},
    protocol,
};
use std::time::Duration;

/// Goggles 2 carry control traffic both ways on this logiclink port.
pub const PORT: u16 = protocol::CONTROL;
/// Gaps between recorded requests are capped. The reference project found
/// 80 ms reliable; shorter bursts left the goggles un-armed.
pub const ARM_GAP_CAP: Duration = Duration::from_millis(80);
/// After arming, do not re-arm sooner than this while no video arrives.
/// Frequent bursts reset the goggles' encoder and ended in USB suspend.
pub const REARM_SETTLE: Duration = Duration::from_secs(20);

pub fn arm() -> &'static [Step] {
    ARM
}
pub fn sustain() -> &'static [Step] {
    SUSTAIN
}

/// Build one framed request with a fresh sequence number. Recorded calendar
/// payloads are replaced with the Pi clock when one is given, so the air unit
/// does not receive the capture date.
pub fn packet(step: &Step, seq: &mut u16, clock: Option<&Wall>) -> Vec<u8> {
    let mut payload = protocol::unhex(step.payload);
    if let Some(clock) = clock {
        patch_calendar(step.set, step.id, &mut payload, clock);
    }
    *seq = seq.wrapping_add(1);
    let flags = (step.ack & 3) << 5;
    protocol::wrap(
        PORT,
        &protocol::duml(
            step.sender,
            step.receiver,
            *seq,
            flags,
            step.set,
            step.id,
            &payload,
        ),
    )
}

/// The capture holds three request types that carry a calendar date
/// (year LE, month, day, hour, minute, second). Overwrite only that field and
/// only when the recorded year (0x07ea = 2026) sits where expected.
fn patch_calendar(set: u8, id: u8, payload: &mut [u8], c: &Wall) {
    let offset = match (set, id) {
        (2, 0x54) | (0, 0x4a) => 0,
        (3, 0xda) if payload.first() == Some(&0x07) => 1,
        _ => return,
    };
    let Some(field) = payload.get_mut(offset..offset + 7) else {
        return;
    };
    if field[..2] != [0xea, 0x07] {
        return;
    }
    field[..2].copy_from_slice(&c.year.to_le_bytes());
    field[2..].copy_from_slice(&[c.month, c.day, c.hour, c.minute, c.second]);
}

/// Delay before `steps[i]`, from the recorded offsets, optionally capped.
pub fn gap(steps: &[Step], i: usize, cap: Option<Duration>) -> Duration {
    let prev = if i == 0 { 0 } else { steps[i - 1].at_ms };
    let d = Duration::from_millis(u64::from(steps[i].at_ms.saturating_sub(prev)));
    cap.map_or(d, |c| d.min(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// First recorded request of this type that carries a 2026 calendar date.
    fn find(set: u8, id: u8) -> &'static Step {
        ARM.iter()
            .find(|s| s.set == set && s.id == id && s.payload.contains("ea07"))
            .unwrap()
    }

    #[test]
    fn rebuilt_request_matches_live_capture() {
        // 0/0x88 to the goggles (type 28, index 1) appears byte-for-byte in the
        // Goggles 2 capture and in the captured vector used by protocol tests.
        let step = ARM
            .iter()
            .find(|s| s.set == 0 && s.id == 0x88 && s.payload.starts_with("1700"))
            .unwrap();
        let mut seq = 0xfef3;
        let framed = packet(step, &mut seq, None);
        assert_eq!(seq, 0xfef4);
        assert_eq!(&framed[..4], &[0x55, 0xcc, 0x30, 0x75]);
        assert_eq!(
            &framed[8..],
            protocol::unhex("551b0475023cf4fe400088170000230041505000000000000258a6")
        );
    }

    #[test]
    fn every_recorded_request_is_valid_duml() {
        let mut seq = 0;
        for step in ARM.iter().chain(SUSTAIN) {
            let framed = packet(step, &mut seq, None);
            assert!(protocol::valid(&framed[8..]), "{}/{:#x}", step.set, step.id);
        }
        assert_eq!(ARM.len(), 218);
        assert_eq!(SUSTAIN.len(), 23);
    }

    #[test]
    fn calendar_requests_use_pi_clock() {
        let clock = Wall {
            unix_ms: 0,
            year: 2027,
            month: 1,
            day: 2,
            hour: 3,
            minute: 4,
            second: 5,
            tz: "UTC".into(),
        };
        let expect = [0xeb, 0x07, 1, 2, 3, 4, 5];
        let mut seq = 0;
        for (set, id, at) in [(2, 0x54, 11), (0, 0x4a, 11), (3, 0xda, 12)] {
            let framed = packet(find(set, id), &mut seq, Some(&clock));
            let duml = &framed[8..];
            assert!(protocol::valid(duml));
            assert_eq!(&duml[at..at + 7], expect, "{set}/{id:#x}");
        }
        // Without a clock the recorded bytes are sent unchanged.
        let framed = packet(find(2, 0x54), &mut seq, None);
        assert_eq!(&framed[8 + 11..8 + 13], [0xea, 0x07]);
        // Unrelated requests, including the dateless 3/0xda, are never touched.
        let dateless = ARM
            .iter()
            .find(|s| s.set == 3 && s.id == 0xda && s.payload == "05ffffffff");
        let framed = packet(dateless.unwrap(), &mut seq, Some(&clock));
        assert_eq!(
            &framed[8 + 11..framed.len() - 2],
            [5, 0xff, 0xff, 0xff, 0xff]
        );
        let other = ARM.iter().find(|s| s.set == 17 && s.id == 0x4a).unwrap();
        let framed = packet(other, &mut seq, Some(&clock));
        assert_eq!(
            &framed[8 + 11..framed.len() - 2],
            protocol::unhex(other.payload)
        );
    }

    #[test]
    fn arming_burst_is_paced_like_the_reference() {
        let total: Duration = (0..ARM.len()).map(|i| gap(ARM, i, Some(ARM_GAP_CAP))).sum();
        assert!(total > Duration::from_secs(2) && total < Duration::from_secs(5));
        let period: Duration = (0..SUSTAIN.len()).map(|i| gap(SUSTAIN, i, None)).sum();
        assert_eq!(period, Duration::from_millis(1010));
    }
}
