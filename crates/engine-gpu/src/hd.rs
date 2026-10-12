//! High-resolution textures from packs: one sRGB texture array per size class plus a parallel
//! array of normal maps. Every texture fills a whole layer (resampled to the class's square),
//! so hardware wrapping, mipmaps and anisotropic filtering just work.

use engine_scene::scene::{HdTexture, RgbaImage};

/// Layer sizes of the five classes.
pub const CLASSES: [u32; 5] = [64, 128, 256, 512, 1024];

/// Where an HD texture lives: class and layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HdSlot {
    pub class: u8,
    pub layer: u16,
    pub normal: bool,
}

pub struct HdSet {
    pub albedo: Vec<wgpu::TextureView>,
    pub normal: Vec<wgpu::TextureView>,
    pub slots: Vec<Option<HdSlot>>,
}

fn class_for(w: usize, h: usize) -> usize {
    let size = w.max(h) as u32;
    CLASSES.iter().position(|&c| c >= size).unwrap_or(CLASSES.len() - 1)
}

/// Bilinear resample to `size`x`size`, wrapping at the edges (tiling textures stay seamless).
fn resample(img: &RgbaImage, size: usize) -> Vec<u8> {
    let (w, h) = (img.width.max(1), img.height.max(1));
    let mut out = vec![0u8; size * size * 4];
    let texel = |x: isize, y: isize| {
        let (x, y) = (x.rem_euclid(w as isize) as usize, y.rem_euclid(h as isize) as usize);
        &img.pixels[(y * w + x) * 4..][..4]
    };
    for y in 0..size {
        for x in 0..size {
            let fx = (x as f32 + 0.5) * w as f32 / size as f32 - 0.5;
            let fy = (y as f32 + 0.5) * h as f32 / size as f32 - 0.5;
            let (x0, y0) = (fx.floor() as isize, fy.floor() as isize);
            let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
            for c in 0..4 {
                let v = |xx, yy| texel(xx, yy)[c] as f32;
                let top = v(x0, y0) * (1.0 - tx) + v(x0 + 1, y0) * tx;
                let bottom = v(x0, y0 + 1) * (1.0 - tx) + v(x0 + 1, y0 + 1) * tx;
                out[(y * size + x) * 4 + c] = (top * (1.0 - ty) + bottom * ty).round() as u8;
            }
        }
    }
    out
}

/// Next mip level (2x2 box filter; colour weighted by alpha so transparent texels don't darken
/// the edges of sprites).
fn downsample(src: &[u8], size: usize) -> Vec<u8> {
    let half = (size / 2).max(1);
    let mut out = vec![0u8; half * half * 4];
    for y in 0..half {
        for x in 0..half {
            let mut acc = [0f32; 4];
            let mut wsum = 0f32;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let o = (((y * 2 + dy).min(size - 1)) * size + (x * 2 + dx).min(size - 1)) * 4;
                let a = src[o + 3] as f32 / 255.0;
                for c in 0..3 {
                    acc[c] += src[o + c] as f32 * a;
                }
                acc[3] += src[o + 3] as f32;
                wsum += a;
            }
            let o = (y * half + x) * 4;
            for c in 0..3 {
                out[o + c] = if wsum > 0.0 { (acc[c] / wsum).round() as u8 } else { 0 };
            }
            out[o + 3] = (acc[3] / 4.0).round() as u8;
        }
    }
    out
}

/// A texture ready for upload: its slot, colour mips and normal-map mips.
type Built = (HdSlot, Vec<Vec<u8>>, Option<Vec<Vec<u8>>>);

/// The image resampled to `size`x`size` and every mip level below it.
fn mip_chain(img: &RgbaImage, size: usize) -> Vec<Vec<u8>> {
    let mut levels = vec![resample(img, size)];
    let mut s = size;
    while s > 1 {
        let next = downsample(levels.last().expect("level"), s);
        levels.push(next);
        s /= 2;
    }
    levels
}

/// `f` over `items` on all CPU cores, keeping the order.
fn parallel<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(items.len().max(1));
    let chunk = items.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = items.chunks(chunk).map(|part| scope.spawn(|| part.iter().map(&f).collect::<Vec<R>>())).collect();
        handles.into_iter().flat_map(|h| h.join().expect("worker panicked")).collect()
    })
}

fn mip_count(size: u32) -> u32 {
    32 - size.leading_zeros()
}

impl HdSet {
    /// Upload the textures (one per image, `None` where the pack has none). Textures beyond the
    /// device's array size stay on the original images.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, textures: &[Option<HdTexture>]) -> Self {
        let max_layers = device.limits().max_texture_array_layers as usize;
        let mut counts = [0usize; CLASSES.len()];
        let slots: Vec<Option<HdSlot>> = textures
            .iter()
            .map(|t| {
                let t = t.as_ref().filter(|t| t.albedo.width > 0 && t.albedo.height > 0)?;
                let class = class_for(t.albedo.width, t.albedo.height);
                (counts[class] < max_layers).then(|| {
                    counts[class] += 1;
                    HdSlot { class: class as u8, layer: (counts[class] - 1) as u16, normal: t.normal.is_some() }
                })
            })
            .collect();
        let make = |label, class: usize, format| {
            let size = CLASSES[class];
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: counts[class].max(1) as u32 },
                mip_level_count: mip_count(size),
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let albedo: Vec<wgpu::Texture> = (0..CLASSES.len()).map(|c| make("hd-albedo", c, wgpu::TextureFormat::Rgba8UnormSrgb)).collect();
        let normal: Vec<wgpu::Texture> = (0..CLASSES.len()).map(|c| make("hd-normal", c, wgpu::TextureFormat::Rgba8Unorm)).collect();
        let upload = |tex: &wgpu::Texture, layer: u32, levels: &Vec<Vec<u8>>| {
            for (level, data) in levels.iter().enumerate() {
                let s = (data.len() / 4).isqrt() as u32;
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: tex, mip_level: level as u32, origin: wgpu::Origin3d { x: 0, y: 0, z: layer }, aspect: wgpu::TextureAspect::All },
                    data,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(s * 4), rows_per_image: Some(s) },
                    wgpu::Extent3d { width: s, height: s, depth_or_array_layers: 1 },
                );
            }
        };
        // Resample and build mip chains on all cores, then upload.
        let jobs: Vec<(usize, &HdTexture, HdSlot)> = textures.iter().zip(&slots).enumerate().filter_map(|(i, (t, s))| Some((i, t.as_ref()?, (*s)?))).collect();
        let built: Vec<Built> = parallel(&jobs, |&(_, t, sl)| {
            let size = CLASSES[sl.class as usize] as usize;
            (sl, mip_chain(&t.albedo, size), t.normal.as_ref().map(|n| mip_chain(n, size)))
        });
        for (sl, colour, normals) in &built {
            upload(&albedo[sl.class as usize], sl.layer as u32, colour);
            if let Some(n) = normals {
                upload(&normal[sl.class as usize], sl.layer as u32, n);
            }
        }
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        Self { albedo: albedo.iter().map(view).collect(), normal: normal.iter().map(view).collect(), slots }
    }

    /// An empty set (one transparent layer per class) so the pipeline always has its bindings.
    pub fn empty(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self::new(device, queue, &[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_resample_and_mips() {
        assert_eq!(class_for(64, 64), 0);
        assert_eq!(class_for(256, 392), 3);
        assert_eq!(class_for(2560, 800), 4, "capped at the largest class");
        assert_eq!(mip_count(256), 9);
        let img = RgbaImage { width: 2, height: 1, pixels: vec![255, 0, 0, 255, 0, 0, 255, 255] };
        let r = resample(&img, 4);
        assert_eq!(r.len(), 64);
        assert!(r.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        // A half-transparent 2x2 averages colour from the opaque texels only.
        let src = [10, 20, 30, 255, 0, 0, 0, 0, 10, 20, 30, 255, 0, 0, 0, 0];
        assert_eq!(downsample(&src, 2), vec![10, 20, 30, 128]);
    }
}
