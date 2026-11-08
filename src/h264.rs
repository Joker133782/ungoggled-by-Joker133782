//! Incremental Annex-B NAL framing, including start codes split across USB reads.
#[derive(Default)]
pub struct AnnexB {
    pending: Vec<u8>,
    scan: usize,
}
impl AnnexB {
    pub fn feed(&mut self, data: &[u8]) -> Result<Vec<Vec<u8>>, &'static str> {
        self.pending.extend_from_slice(data);
        let mut units = Vec::new();
        let mut start = 0;
        let mut found = self.pending.starts_with(&[0, 0, 1]);
        let mut scan = self.scan.max(if found { 3 } else { 0 });
        while scan + 3 <= self.pending.len() {
            if self.pending[scan..scan + 3] == [0, 0, 1] {
                if found && scan > start + 3 {
                    if scan - start > 2_000_000 {
                        self.pending.clear();
                        self.scan = 0;
                        return Err("H.264 NAL exceeds 2 MB; waiting for fresh SPS");
                    }
                    units.push(self.pending[start..scan].to_vec());
                }
                start = scan;
                found = true;
                scan += 3;
            } else {
                scan += 1;
            }
        }
        if found {
            self.pending.drain(..start);
            self.scan = scan - start;
        } else {
            // Retain only a possible incomplete start code, not arbitrary junk.
            let keep = self.pending.len().min(2);
            self.pending.drain(..self.pending.len() - keep);
            self.scan = 0;
        }
        if self.pending.len() > 2_000_000 {
            self.pending.clear();
            self.scan = 0;
            return Err("H.264 NAL exceeds 2 MB; waiting for fresh SPS");
        }
        Ok(units)
    }
}

/// Repeated SPSs are normal. Only changed codec parameters need a new decoder.
#[derive(Default)]
pub struct Parameters(Option<Vec<u8>>);
impl Parameters {
    pub fn changed(&mut self, nal: &[u8]) -> bool {
        if nal.len() < 4 || nal[3] & 31 != 7 {
            return false;
        }
        let mut sps = &nal[3..];
        while sps.last() == Some(&0) {
            sps = &sps[..sps.len() - 1];
        }
        let changed = self.0.as_ref().is_some_and(|old| old != sps);
        self.0 = Some(sps.to_vec());
        changed
    }
}

/// Reads bits (most significant first) from an RBSP, i.e. a NAL payload with
/// the emulation-prevention bytes already removed.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}
impl Bits<'_> {
    fn bit(&mut self) -> Option<u32> {
        let byte = *self.data.get(self.pos / 8)?;
        let bit = (byte >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Some(u32::from(bit))
    }
    fn bits(&mut self, n: u32) -> Option<u32> {
        (0..n).try_fold(0_u32, |v, _| Some(v << 1 | self.bit()?))
    }
    /// Unsigned Exp-Golomb code.
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        ((1_u64 << zeros) - 1 + u64::from(self.bits(zeros)?))
            .try_into()
            .ok()
    }
    /// Signed Exp-Golomb code.
    fn se(&mut self) -> Option<i32> {
        let k = i64::from(self.ue()?);
        let v = if k % 2 == 1 { (k + 1) / 2 } else { -(k / 2) };
        i32::try_from(v).ok()
    }
}

/// Picture size in pixels (after cropping) from a sequence parameter set
/// NAL as `AnnexB` delivers it (`00 00 01`, header byte, payload). `None` for
/// anything that is not a well-formed SPS.
pub fn sps_size(nal: &[u8]) -> Option<(u32, u32)> {
    if nal.len() < 8 || nal[3] & 31 != 7 {
        return None;
    }
    let mut rbsp = Vec::with_capacity(nal.len());
    let mut zeros = 0;
    for &b in &nal[4..] {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        rbsp.push(b);
    }
    let mut r = Bits {
        data: &rbsp,
        pos: 0,
    };
    let profile = r.bits(8)?;
    r.bits(16)?; // constraint flags, level
    r.ue()?; // seq_parameter_set_id
    let mut chroma_format = 1;
    if matches!(
        profile,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    ) {
        chroma_format = r.ue()?;
        if chroma_format > 3 {
            return None;
        }
        if chroma_format == 3 {
            r.bit()?; // separate_colour_plane_flag
        }
        r.ue()?; // bit depth luma
        r.ue()?; // bit depth chroma
        r.bit()?; // qpprime_y_zero_transform_bypass_flag
        if r.bit()? == 1 {
            // Scaling matrices: only their length matters here.
            for i in 0..if chroma_format == 3 { 12 } else { 8 } {
                if r.bit()? == 1 {
                    let (mut last, mut next) = (8_i32, 8_i32);
                    for _ in 0..if i < 6 { 16 } else { 64 } {
                        if next != 0 {
                            next = (last + r.se()? + 256).rem_euclid(256);
                        }
                        if next != 0 {
                            last = next;
                        }
                    }
                }
            }
        }
    }
    r.ue()?; // log2_max_frame_num_minus4
    match r.ue()? {
        0 => {
            r.ue()?;
        }
        1 => {
            r.bit()?;
            r.se()?;
            r.se()?;
            for _ in 0..r.ue()?.min(255) {
                r.se()?;
            }
        }
        _ => {}
    }
    r.ue()?; // max_num_ref_frames
    r.bit()?; // gaps_in_frame_num_value_allowed_flag
    let width_mbs = r.ue()?.checked_add(1)?;
    let height_units = r.ue()?.checked_add(1)?;
    let frame_mbs_only = r.bit()?;
    if frame_mbs_only == 0 {
        r.bit()?; // mb_adaptive_frame_field_flag
    }
    r.bit()?; // direct_8x8_inference_flag
    let mut crop = [0_u32; 4]; // left, right, top, bottom
    if r.bit()? == 1 {
        for c in &mut crop {
            *c = r.ue()?;
        }
    }
    let unit_x = if chroma_format == 1 || chroma_format == 2 {
        2
    } else {
        1
    };
    let unit_y = (if chroma_format == 1 { 2 } else { 1 }) * (2 - frame_mbs_only);
    let width = width_mbs
        .checked_mul(16)?
        .checked_sub(unit_x * (crop[0].checked_add(crop[1])?))?;
    let height = height_units
        .checked_mul(16 * (2 - frame_mbs_only))?
        .checked_sub(unit_y * (crop[2].checked_add(crop[3])?))?;
    // Nothing this app decodes is smaller than 64 px; tiny sizes mean junk.
    ((64..=16_384).contains(&width) && (64..=16_384).contains(&height)).then_some((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Bit writer for building SPS test vectors.
    #[derive(Default)]
    struct Writer(Vec<bool>);
    impl Writer {
        fn put(&mut self, v: u32, n: u32) {
            for i in (0..n).rev() {
                self.0.push(v >> i & 1 == 1);
            }
        }
        fn ue(&mut self, v: u32) {
            let x = v + 1;
            let n = 32 - x.leading_zeros();
            self.put(0, n - 1);
            self.put(x, n);
        }
        /// The finished NAL: start code, header, payload with emulation
        /// prevention applied.
        fn nal(mut self) -> Vec<u8> {
            self.0.push(true);
            while !self.0.len().is_multiple_of(8) {
                self.0.push(false);
            }
            let payload: Vec<u8> = self
                .0
                .chunks(8)
                .map(|c| c.iter().fold(0, |v, &b| v << 1 | u8::from(b)))
                .collect();
            let mut nal = vec![0, 0, 1, 0x67];
            let mut zeros = 0;
            for b in payload {
                if zeros >= 2 && b <= 3 {
                    nal.push(3);
                    zeros = 0;
                }
                zeros = if b == 0 { zeros + 1 } else { 0 };
                nal.push(b);
            }
            nal
        }
    }
    fn sps(profile: u32, mbs: (u32, u32), frame_mbs_only: bool, crop: [u32; 4]) -> Vec<u8> {
        let mut w = Writer::default();
        w.put(profile, 8);
        w.put(0, 8);
        w.put(40, 8);
        w.ue(0);
        if profile >= 100 {
            w.ue(1); // 4:2:0
            w.ue(0);
            w.ue(0);
            w.put(0, 2); // no bypass, no scaling matrix
        }
        w.ue(0); // log2_max_frame_num_minus4
        w.ue(2); // pic_order_cnt_type 2
        w.ue(1); // max_num_ref_frames
        w.put(0, 1);
        w.ue(mbs.0 - 1);
        w.ue(mbs.1 - 1);
        w.put(u32::from(frame_mbs_only), 1);
        if !frame_mbs_only {
            w.put(0, 1);
        }
        w.put(1, 1); // direct_8x8_inference
        if crop == [0; 4] {
            w.put(0, 1);
        } else {
            w.put(1, 1);
            for c in crop {
                w.ue(c);
            }
        }
        w.put(0, 1); // no VUI
        w.nal()
    }
    #[test]
    fn sps_size_of_plain_and_cropped_pictures() {
        assert_eq!(sps_size(&sps(66, (40, 30), true, [0; 4])), Some((640, 480)));
        // 1080p is coded as 1088 lines and cropped by 4 chroma units.
        assert_eq!(
            sps_size(&sps(100, (120, 68), true, [0, 0, 0, 4])),
            Some((1920, 1080))
        );
        assert_eq!(
            sps_size(&sps(100, (90, 68), true, [0, 0, 0, 4])),
            Some((1440, 1080))
        );
        // Interlaced: a map unit is two macroblock rows, a crop unit four lines.
        assert_eq!(
            sps_size(&sps(100, (120, 34), false, [0, 0, 0, 2])),
            Some((1920, 1080))
        );
        assert_eq!(
            sps_size(&sps(100, (120, 68), true, [2, 6, 0, 0])),
            Some((1920 - 16, 1088))
        );
    }
    #[test]
    fn sps_size_of_a_real_encoder() {
        // x264, High profile, 1080p, with emulation-prevention bytes inside.
        let sps = [
            0, 0, 1, 0x67, 0x64, 0x00, 0x28, 0xAC, 0xD9, 0x40, 0x78, 0x02, 0x27, 0xE5, 0xC0, 0x44,
            0x00, 0x00, 0x03, 0x00, 0x04, 0x00, 0x00, 0x03, 0x00, 0xF0, 0x3C, 0x60, 0xC6, 0x58,
        ];
        assert_eq!(sps_size(&sps), Some((1920, 1080)));
    }
    #[test]
    fn sps_size_of_the_built_in_loop() {
        let data = include_bytes!("../assets/default-loop.h264");
        let mut annex = AnnexB::default();
        let nals = annex.feed(&data[..200_000.min(data.len())]).unwrap();
        let sps = nals
            .iter()
            .find(|n| n.get(3).is_some_and(|b| b & 31 == 7))
            .unwrap();
        let (w, h) = sps_size(sps).unwrap();
        assert!((w, h) == (1920, 1080) || (w, h) == (1280, 720), "{w}x{h}");
    }
    #[test]
    fn sps_size_rejects_other_and_broken_input() {
        assert_eq!(sps_size(&[]), None);
        assert_eq!(sps_size(&[0, 0, 1, 0x68, 1, 2, 3, 4, 5, 6]), None);
        let good = sps(100, (120, 68), true, [0, 0, 0, 4]);
        for cut in 4..good.len() - 1 {
            assert_ne!(sps_size(&good[..cut]), Some((1920, 1080)), "cut at {cut}");
        }
        assert_eq!(
            sps_size(&[0, 0, 1, 0x67, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
            None
        );
    }
    #[test]
    fn every_usb_split_preserves_nals_and_emulation_prevention() {
        let bytes = [
            0, 0, 1, 0x67, 100, 0, 52, 0x80, 0, 0, 0, 1, 0x68, 0x80, 0, 0, 1, 0x65, 0, 0, 3, 1,
            0x80, 0, 0, 1, 9, 0x10,
        ];
        let expected = vec![
            bytes[0..9].to_vec(),
            bytes[9..14].to_vec(),
            bytes[14..23].to_vec(),
        ];
        for size in 1..=bytes.len() {
            let mut parser = AnnexB::default();
            let actual: Vec<_> = bytes
                .chunks(size)
                .flat_map(|c| parser.feed(c).unwrap())
                .collect();
            assert_eq!(actual, expected, "USB chunk size {size}");
        }
    }
    #[test]
    fn only_changed_sps_restarts_decoder() {
        let mut parameters = Parameters::default();
        let sps = [0, 0, 1, 0x67, 100, 0, 52, 0x80];
        assert!(!parameters.changed(&sps));
        assert!(!parameters.changed(&[0, 0, 1, 0x68, 0x80]));
        assert!(!parameters.changed(&[sps.as_slice(), &[0]].concat()));
        let mut changed = sps;
        changed[6] = 42;
        assert!(parameters.changed(&changed));
        assert!(!parameters.changed(&changed));
    }
    #[test]
    fn garbage_and_oversize_input_are_bounded() {
        let mut parser = AnnexB::default();
        assert!(parser.feed(&[8; 1000]).unwrap().is_empty());
        assert!(parser.pending.len() <= 2);
        parser.feed(&[0, 0, 1, 0x65]).unwrap();
        assert!(parser.feed(&vec![1; 2_000_001]).is_err());
        assert!(parser.pending.is_empty());
        let mut complete = vec![0, 0, 1, 0x65];
        complete.extend(vec![1; 2_000_001]);
        complete.extend([0, 0, 1, 0x67]);
        assert!(parser.feed(&complete).is_err());
        assert!(parser.pending.is_empty());
    }
}
