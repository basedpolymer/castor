//! Gradient noise (improved Perlin, 3D) with a seeded permutation, fBm and domain warping.
//! Shared by gradients, shaders, patterns and the mockup backgrounds.

#[derive(Clone)]
pub struct Noise {
    p: [u8; 512],
}

fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn grad(h: u8, x: f32, y: f32, z: f32) -> f32 {
    let h = h & 15;
    let u = if h < 8 { x } else { y };
    let v = if h < 4 {
        y
    } else if h == 12 || h == 14 {
        x
    } else {
        z
    };
    (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
}

impl Noise {
    pub fn new(seed: u64) -> Noise {
        let mut perm: Vec<u8> = (0..=255).collect();
        let mut rng = crate::common::Rng::new(seed);
        for i in (1..256).rev() {
            let j = (rng.next_u64() % (i as u64 + 1)) as usize;
            perm.swap(i, j);
        }
        let mut p = [0u8; 512];
        for i in 0..512 {
            p[i] = perm[i & 255];
        }
        Noise { p }
    }

    /// Noise in about [-1, 1].
    pub fn get(&self, x: f32, y: f32, z: f32) -> f32 {
        let (xf, yf, zf) = (x.floor(), y.floor(), z.floor());
        let (xi, yi, zi) = ((xf as i32 & 255) as usize, (yf as i32 & 255) as usize, (zf as i32 & 255) as usize);
        let (x, y, z) = (x - xf, y - yf, z - zf);
        let (u, v, w) = (fade(x), fade(y), fade(z));
        let p = &self.p;
        let a = p[xi] as usize + yi;
        let aa = p[a] as usize + zi;
        let ab = p[a + 1] as usize + zi;
        let b = p[xi + 1] as usize + yi;
        let ba = p[b] as usize + zi;
        let bb = p[b + 1] as usize + zi;
        lerp(
            lerp(
                lerp(grad(p[aa], x, y, z), grad(p[ba], x - 1.0, y, z), u),
                lerp(grad(p[ab], x, y - 1.0, z), grad(p[bb], x - 1.0, y - 1.0, z), u),
                v,
            ),
            lerp(
                lerp(grad(p[aa + 1], x, y, z - 1.0), grad(p[ba + 1], x - 1.0, y, z - 1.0), u),
                lerp(grad(p[ab + 1], x, y - 1.0, z - 1.0), grad(p[bb + 1], x - 1.0, y - 1.0, z - 1.0), u),
                v,
            ),
            w,
        )
    }

    /// Fractal Brownian motion, `oct` octaves, about [-1, 1].
    pub fn fbm(&self, x: f32, y: f32, z: f32, oct: u32) -> f32 {
        let (mut s, mut a, mut f, mut norm) = (0.0, 1.0, 1.0, 0.0);
        for _ in 0..oct.max(1) {
            s += a * self.get(x * f, y * f, z * f);
            norm += a;
            a *= 0.5;
            f *= 2.03;
        }
        s / norm
    }
}

/// Smoothstep.
pub fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Hash of integer coordinates to [0, 1) (grain, sparkles).
pub fn hash2(x: u32, y: u32, seed: u32) -> f32 {
    let mut h = x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 16_777_216.0
}
