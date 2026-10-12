//! Upscalers. `scale2x` (EPX, applied twice for 4x) is built in and keeps pixel art crisp;
//! `esrgan` runs `realesrgan-ncnn-vulkan` (Real-ESRGAN on the GPU) per class: tiling classes
//! are padded with wrapped texels and cropped afterwards so their edges stay seamless, and
//! transparent images have their colours dilated into the transparent texels (no dark halos)
//! while the alpha mask is scaled separately with scale2x and kept binary.

use super::{AnyResult, Class};
use engine_core::pack::{Rgba, decode_png, encode_png};

/// Wrapped texels added around tiling images before ESRGAN (at the original size).
const PAD: usize = 8;

pub fn esrgan_available() -> bool {
    std::process::Command::new("realesrgan-ncnn-vulkan").arg("-h").output().is_ok()
}

pub fn upscale_all(upscaler: &str, images: &[Rgba], classes: &[Class], scale: usize) -> AnyResult<Vec<Rgba>> {
    match upscaler {
        "scale2x" => Ok(images.iter().zip(classes).map(|(img, c)| scale4x(img, c.tiles(), scale)).collect()),
        "esrgan" => esrgan(images, classes, scale),
        other => Err(format!("unknown upscaler {other:?} (scale2x or esrgan)").into()),
    }
}

/// scale2x applied until the image is `scale` times larger (scale must be a power of two).
fn scale4x(img: &Rgba, wrap: bool, scale: usize) -> Rgba {
    let mut out = img.clone();
    let mut s = 1;
    while s < scale {
        out = scale2x(&out, wrap);
        s *= 2;
    }
    out
}

/// EPX / Scale2x: each texel becomes 2x2, corners take a neighbour's colour where two
/// neighbours agree, so diagonal edges stay sharp instead of blocky.
pub fn scale2x(img: &Rgba, wrap: bool) -> Rgba {
    let (w, h) = (img.width, img.height);
    let px = |x: isize, y: isize| -> [u8; 4] {
        let (x, y) = if wrap {
            (x.rem_euclid(w as isize) as usize, y.rem_euclid(h as isize) as usize)
        } else {
            (x.clamp(0, w as isize - 1) as usize, y.clamp(0, h as isize - 1) as usize)
        };
        let o = (y * w + x) * 4;
        [img.pixels[o], img.pixels[o + 1], img.pixels[o + 2], img.pixels[o + 3]]
    };
    let mut out = vec![0u8; w * h * 16];
    for y in 0..h as isize {
        for x in 0..w as isize {
            let p = px(x, y);
            let (a, b, c, d) = (px(x, y - 1), px(x + 1, y), px(x - 1, y), px(x, y + 1));
            let mut e = [p; 4];
            if c == a && c != d && a != b {
                e[0] = a;
            }
            if a == b && a != c && b != d {
                e[1] = b;
            }
            if d == c && d != b && c != a {
                e[2] = c;
            }
            if b == d && b != a && d != c {
                e[3] = d;
            }
            for (k, q) in e.iter().enumerate() {
                let (ox, oy) = (x as usize * 2 + k % 2, y as usize * 2 + k / 2);
                let o = (oy * w * 2 + ox) * 4;
                out[o..o + 4].copy_from_slice(q);
            }
        }
    }
    Rgba { width: w * 2, height: h * 2, pixels: out }
}

/// Spread opaque colours into transparent texels (repeatedly averaging opaque neighbours), so
/// filtering and upscaling never pull in the black behind index 0.
fn dilate(img: &Rgba) -> Rgba {
    let (w, h) = (img.width, img.height);
    let mut out = img.clone();
    let mut known: Vec<bool> = img.pixels.as_chunks::<4>().0.iter().map(|p| p[3] > 0).collect();
    for _ in 0..8 {
        let prev = out.clone();
        let mut next = known.clone();
        for y in 0..h {
            for x in 0..w {
                if known[y * w + x] {
                    continue;
                }
                let (mut sum, mut n) = ([0u32; 3], 0);
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    let (nx, ny) = (x as isize + dx, y as isize + dy);
                    if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize || !known[ny as usize * w + nx as usize] {
                        continue;
                    }
                    let o = (ny as usize * w + nx as usize) * 4;
                    for (s, &v) in sum.iter_mut().zip(&prev.pixels[o..o + 3]) {
                        *s += v as u32;
                    }
                    n += 1;
                }
                if n > 0 {
                    let o = (y * w + x) * 4;
                    for (d, s) in out.pixels[o..o + 3].iter_mut().zip(sum) {
                        *d = s.checked_div(n).unwrap_or(0) as u8;
                    }
                    next[y * w + x] = true;
                }
            }
        }
        known = next;
    }
    out
}

/// Tile `img` with `pad` wrapped texels on every side.
fn pad_wrapped(img: &Rgba, pad: usize) -> Rgba {
    let (w, h) = (img.width, img.height);
    let (pw, ph) = (w + 2 * pad, h + 2 * pad);
    let mut pixels = vec![0u8; pw * ph * 4];
    for y in 0..ph {
        for x in 0..pw {
            let sx = (x as isize - pad as isize).rem_euclid(w as isize) as usize;
            let sy = (y as isize - pad as isize).rem_euclid(h as isize) as usize;
            pixels[(y * pw + x) * 4..][..4].copy_from_slice(&img.pixels[(sy * w + sx) * 4..][..4]);
        }
    }
    Rgba { width: pw, height: ph, pixels }
}

fn crop(img: &Rgba, x0: usize, y0: usize, w: usize, h: usize) -> Rgba {
    let mut pixels = Vec::with_capacity(w * h * 4);
    for y in y0..y0 + h {
        pixels.extend_from_slice(&img.pixels[(y * img.width + x0) * 4..][..w * 4]);
    }
    Rgba { width: w, height: h, pixels }
}

fn esrgan(images: &[Rgba], classes: &[Class], scale: usize) -> AnyResult<Vec<Rgba>> {
    let tmp = std::env::temp_dir().join(format!("packbuilder-{}", std::process::id()));
    let model_of = |c: Class| if matches!(c, Class::Sprite | Class::Other) { "realesrgan-x4plus-anime" } else { "realesrgan-x4plus" };
    let mut results: Vec<Option<Rgba>> = vec![None; images.len()];
    for model in ["realesrgan-x4plus", "realesrgan-x4plus-anime"] {
        let (input, output) = (tmp.join(model).join("in"), tmp.join(model).join("out"));
        std::fs::create_dir_all(&input)?;
        std::fs::create_dir_all(&output)?;
        let members: Vec<usize> = (0..images.len()).filter(|&i| model_of(classes[i]) == model).collect();
        for &i in &members {
            let mut src = if classes[i].transparent() { dilate(&images[i]) } else { images[i].clone() };
            src.pixels.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p[3] = 255);
            if classes[i].tiles() {
                src = pad_wrapped(&src, PAD);
            }
            std::fs::write(input.join(format!("{i}.png")), encode_png(&src)?)?;
        }
        if members.is_empty() {
            continue;
        }
        println!("realesrgan-ncnn-vulkan: {} images with {model}", members.len());
        let status = std::process::Command::new("realesrgan-ncnn-vulkan")
            .args(["-i", &input.to_string_lossy(), "-o", &output.to_string_lossy(), "-n", model, "-s", &scale.to_string(), "-f", "png"])
            .status()?;
        if !status.success() {
            return Err(format!("realesrgan-ncnn-vulkan failed ({status})").into());
        }
        for &i in &members {
            let mut up = decode_png(&std::fs::read(output.join(format!("{i}.png")))?)?;
            if classes[i].tiles() {
                up = crop(&up, PAD * scale, PAD * scale, images[i].width * scale, images[i].height * scale);
            }
            // Alpha from the crisp pixel-art scaler, so edges stay binary and aligned.
            let mask = scale4x(&images[i], classes[i].tiles(), scale);
            for (p, m) in up.pixels.as_chunks_mut::<4>().0.iter_mut().zip(mask.pixels.as_chunks::<4>().0.iter()) {
                p[3] = m[3];
            }
            results[i] = Some(up);
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    results.into_iter().enumerate().map(|(i, r)| r.ok_or_else(|| format!("image {i} was not upscaled").into())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 4]) -> Rgba {
        let f = &f;
        Rgba { width: w, height: h, pixels: (0..h).flat_map(|y| (0..w).flat_map(move |x| f(x, y))).collect() }
    }

    #[test]
    fn scale2x_keeps_flat_areas_and_sharpens_diagonals() {
        let flat = img(3, 3, |_, _| [9, 9, 9, 255]);
        assert!(scale2x(&flat, false).pixels.as_chunks::<4>().0.iter().all(|p| *p == [9, 9, 9, 255]));
        // A diagonal: black above the anti-diagonal, white below.
        let diag = img(4, 4, |x, y| if x + y < 4 { [0, 0, 0, 255] } else { [255, 255, 255, 255] });
        let up = scale2x(&diag, false);
        assert_eq!((up.width, up.height), (8, 8));
        let distinct = up.pixels.as_chunks::<4>().0.iter().filter(|p| p[0] == 0).count();
        assert!(distinct > 4 * 10 * 4 / 4 / 2, "edge got finer, not lost");
    }

    #[test]
    fn padding_wraps_and_crop_restores() {
        let src = img(5, 3, |x, y| [x as u8, y as u8, 0, 255]);
        let padded = pad_wrapped(&src, 2);
        assert_eq!((padded.width, padded.height), (9, 7));
        assert_eq!(&padded.pixels[0..4], &[3, 1, 0, 255], "top-left comes from the opposite corner");
        assert_eq!(crop(&padded, 2, 2, 5, 3), src);
    }

    #[test]
    fn dilation_fills_transparent_colour() {
        let src = img(3, 1, |x, _| if x == 0 { [200, 100, 50, 255] } else { [0, 0, 0, 0] });
        let d = dilate(&src);
        assert_eq!(&d.pixels[8..11], &[200, 100, 50]);
        assert_eq!(d.pixels[11], 0, "alpha untouched");
    }
}
