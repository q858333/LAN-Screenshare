pub fn scale_rgba_nearest(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    if src_w == dst_w && src_h == dst_h {
        return src.to_vec();
    }
    let mut dst = vec![0u8; (dst_w * dst_h * 4) as usize];
    for y in 0..dst_h {
        for x in 0..dst_w {
            let sx = (x as u64 * src_w as u64 / dst_w as u64) as u32;
            let sy = (y as u64 * src_h as u64 / dst_h as u64) as u32;
            let si = ((sy * src_w + sx) * 4) as usize;
            let di = ((y * dst_w + x) * 4) as usize;
            dst[di..di + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
    dst
}

pub fn fit_width_dims(src_w: u32, src_h: u32, max_width: u32) -> (u32, u32) {
    if src_w <= max_width {
        return (src_w & !1, src_h & !1);
    }
    let dst_w = max_width & !1;
    let dst_h = ((src_h as u64 * dst_w as u64) / src_w as u64) as u32 & !1;
    (dst_w.max(2), dst_h.max(2))
}
