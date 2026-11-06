//! Estimate the quality setting of an existing JPEG from its luminance quantisation table, so
//! that edited JPEGs are saved at the quality they came with (no bloat, no extra loss).

/// Standard IJG luminance table (zig-zag order is irrelevant for the ratio we compute).
const STD_LUMA: [u16; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56, 14, 17, 22, 29, 51, 87, 80,
    62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113, 92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98,
    112, 100, 103, 99,
];

/// IJG-equivalent quality 1..=100, or None if no DQT segment was found.
pub fn estimate(jpeg: &[u8]) -> Option<u8> {
    let mut i = 2;
    while i + 4 <= jpeg.len() {
        if jpeg[i] != 0xFF {
            return None;
        }
        let marker = jpeg[i + 1];
        if marker == 0xDA {
            return None;
        }
        let len = u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]) as usize;
        if marker == 0xDB {
            let seg = jpeg.get(i + 4..i + 2 + len)?;
            let pq = seg[0] >> 4;
            let tq = seg[0] & 0x0F;
            if tq == 0 {
                let vals: Vec<u16> = if pq == 0 {
                    seg.get(1..65)?.iter().map(|v| *v as u16).collect()
                } else {
                    seg.get(1..129)?.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect()
                };
                let sum: f64 = vals.iter().map(|v| *v as f64).sum();
                let std: f64 = STD_LUMA.iter().map(|v| *v as f64).sum();
                let scale = sum / std * 100.0; // IJG scale factor in percent
                let q = if scale <= 100.0 { (200.0 - scale) / 2.0 } else { 5000.0 / scale };
                return Some(q.round().clamp(1.0, 100.0) as u8);
            }
        }
        i += 2 + len;
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn roundtrip_quality() {
        for q in [50u8, 75, 90] {
            let img = image::RgbImage::from_fn(64, 64, |x, y| image::Rgb([(x * 4) as u8, (y * 4) as u8, 128]));
            let mut buf = std::io::Cursor::new(Vec::new());
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, q).encode_image(&img).unwrap();
            let est = super::estimate(buf.get_ref()).unwrap();
            assert!((est as i32 - q as i32).abs() <= 2, "{q} → {est}");
        }
    }
}
