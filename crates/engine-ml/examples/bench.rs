//! Quick benchmark / smoke test of the ML engine.
//!
//! ```text
//! cargo run -p engine-ml --release --example bench -- bg isnet-general-use photo.jpg out.png
//! cargo run -p engine-ml --release --example bench -- up realesrgan-x4plus small.png out.png 4
//! cargo run -p engine-ml --release --example bench -- faces yunet group.jpg out.png
//! ```
//! `CASTOR_ML_BACKEND=tract` forces the pure-Rust backend; `CASTOR_HOME` selects where models are.

use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(a.len() >= 4, "usage: bench <bg|up|faces> <model> <in> <out> [scale]");
    let cx = castor_core::RunCx::new(std::env::temp_dir(), std::env::temp_dir(), "", castor_core::CancelToken::new())
        .on_message(|m| eprintln!("{m}"));
    let img = image::open(&a[2])?;
    eprintln!("backend: {}  input: {}x{}", engine_ml::backend().name(), img.width(), img.height());
    match a[0].as_str() {
        "bg" => {
            let rgba = img.to_rgba8();
            for run in 0..2 {
                let t = Instant::now();
                let m = engine_ml::remove_background(&rgba, &a[1], &cx)?;
                eprintln!("run {run}: {:?}", t.elapsed());
                if run == 1 {
                    m.save(format!("{}.matte.png", a[3]))?;
                    let mut out = rgba.clone();
                    engine_ml::matting::decontaminate(&mut out, &m);
                    for (p, q) in out.pixels_mut().zip(m.pixels()) {
                        p[3] = q[0];
                    }
                    out.save(&a[3])?;
                }
            }
        }
        "up" => {
            let scale: u32 = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(4);
            let t = Instant::now();
            let out = engine_ml::upscale(&img.to_rgb8(), scale, &a[1], &cx)?;
            eprintln!("upscale: {:?} → {}x{}", t.elapsed(), out.width(), out.height());
            out.save(&a[3])?;
        }
        "faces" => {
            let rgb = img.to_rgb8();
            let t = Instant::now();
            let faces = engine_ml::detect_faces(&rgb, &cx)?;
            eprintln!("faces: {} in {:?}", faces.len(), t.elapsed());
            let mut out = img.to_rgb8();
            for f in &faces {
                let (x0, y0, x1, y1) = (f[0] as u32, f[1] as u32, (f[2] as u32).min(out.width() - 1), (f[3] as u32).min(out.height() - 1));
                for x in x0..=x1 {
                    for y in [y0, y1] {
                        out.put_pixel(x, y, image::Rgb([255, 0, 0]));
                    }
                }
                for y in y0..=y1 {
                    for x in [x0, x1] {
                        out.put_pixel(x, y, image::Rgb([255, 0, 0]));
                    }
                }
            }
            out.save(&a[3])?;
        }
        other => anyhow::bail!("unknown task {other}"),
    }
    Ok(())
}
