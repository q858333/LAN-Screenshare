pub fn rgba_to_i420(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let y_size = w * h;
    let uv_size = (w / 2) * (h / 2);
    let mut out = vec![0u8; y_size + uv_size * 2];

    let (y_plane, rest) = out.split_at_mut(y_size);
    let (u_plane, v_plane) = rest.split_at_mut(uv_size);

    for row in 0..h {
        for col in 0..w {
            let i = (row * w + col) * 4;
            let r = rgba[i] as f32;
            let g = rgba[i + 1] as f32;
            let b = rgba[i + 2] as f32;
            y_plane[row * w + col] = (0.257 * r + 0.504 * g + 0.098 * b + 16.0).clamp(0.0, 255.0) as u8;
        }
    }

    for row in (0..h).step_by(2) {
        for col in (0..w).step_by(2) {
            let mut r_sum = 0f32;
            let mut g_sum = 0f32;
            let mut b_sum = 0f32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let rr = row + dy;
                    let cc = col + dx;
                    if rr >= h || cc >= w {
                        continue;
                    }
                    let i = (rr * w + cc) * 4;
                    r_sum += rgba[i] as f32;
                    g_sum += rgba[i + 1] as f32;
                    b_sum += rgba[i + 2] as f32;
                }
            }
            let r = r_sum / 4.0;
            let g = g_sum / 4.0;
            let b = b_sum / 4.0;
            let uv_row = row / 2;
            let uv_col = col / 2;
            let uv_idx = uv_row * (w / 2) + uv_col;
            u_plane[uv_idx] = (-0.148 * r - 0.291 * g + 0.439 * b + 128.0).clamp(0.0, 255.0) as u8;
            v_plane[uv_idx] = (0.439 * r - 0.368 * g - 0.071 * b + 128.0).clamp(0.0, 255.0) as u8;
        }
    }

    out
}
