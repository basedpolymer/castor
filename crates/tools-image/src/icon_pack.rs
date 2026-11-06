//! image.icon-pack — one master image → ICO, ICNS, favicon set + site.webmanifest, iOS AppIcon
//! set (Contents.json), Android mipmaps, in a folder.

use std::path::Path;

use anyhow::{Context, Result};
use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{FileInfo, Job, Output, Preview, RunCx, Tool, ToolMeta, ToolStatus};
use image::{DynamicImage, Rgba, RgbaImage};

use crate::codec::{self, DecodeOpts, EncodeOpts, Fmt};
use crate::{common, ops};

pub struct IconPack {
    meta: ToolMeta,
}

impl IconPack {
    pub fn new() -> Self {
        IconPack {
            meta: common::meta(
                "image.icon-pack",
                "ICO · ICNS · PNG",
                &["icon", "favicon", "ico", "icns", "app icon", "pwa", "manifest", "android", "ios", "apple-touch-icon"],
                "icons",
                ToolStatus::Ready,
                &[],
                false,
            ),
        }
    }
}

impl Tool for IconPack {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice(
                "targets",
                vec![
                    Choice::key("all"),
                    Choice::key("web"),
                    Choice::lit("ico", "ICO"),
                    Choice::lit("icns", "ICNS"),
                    Choice::lit("ios", "iOS"),
                    Choice::lit("android", "Android"),
                ],
                "all",
            ),
            OptionSpec::int("padding", 0, 40, 0).slider().unit("%"),
            OptionSpec::color("background", "#00000000"),
            OptionSpec::int("radius", 0, 50, 0).slider().unit("%"),
            OptionSpec::text("app_name", "").advanced(),
            OptionSpec::color("theme_color", "#ffffff").advanced(),
        ]
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let d = common::load(input, DecodeOpts { svg_side: 1024, ..Default::default() })?;
        let master = master(&d.image.to_rgba8(), v, 1024);
        cx.progress(0.1);
        let dir = cx.output_named(&format!("{}-icons", input.stem()));
        std::fs::create_dir_all(&dir).with_context(|| common::err("err.write", &dir))?;
        let t = v.str("targets");
        let all = t == "all";
        let opaque_bg = {
            let c = v.color("background");
            if c[3] == 255 { c } else { [255, 255, 255, 255] }
        };
        let mut count = 0usize;
        let png = |name: &Path, img: &RgbaImage| -> Result<()> {
            let b = codec::encode(&DynamicImage::ImageRgba8(img.clone()), Fmt::Png, &EncodeOpts { png_effort: 2, ..Default::default() })?;
            if let Some(p) = name.parent() {
                std::fs::create_dir_all(p)?;
            }
            common::write(name, &b)
        };
        let sized = |s: u32| ops::resize_rgba(&master, s, s);
        // ICO (Windows): 16…256.
        if all || t == "ico" || t == "web" {
            let sizes: &[u32] = if t == "web" { &[16, 32, 48] } else { &[16, 24, 32, 48, 64, 128, 256] };
            let name = if t == "web" || all { "favicon.ico" } else { "icon.ico" };
            let ico = codec::ico_from_sizes(&DynamicImage::ImageRgba8(master.clone()), sizes)?;
            common::write(&dir.join(name), &ico)?;
            count += 1;
            if all {
                let ico = codec::ico_from_sizes(&DynamicImage::ImageRgba8(master.clone()), &[16, 24, 32, 48, 64, 128, 256])?;
                common::write(&dir.join("icon.ico"), &ico)?;
                count += 1;
            }
        }
        cx.progress(0.3);
        // ICNS (macOS).
        if all || t == "icns" {
            use icns::IconType as T;
            let mut fam = icns::IconFamily::new();
            for (ty, s) in [
                (T::RGBA32_16x16, 16),
                (T::RGBA32_16x16_2x, 32),
                (T::RGBA32_32x32, 32),
                (T::RGBA32_32x32_2x, 64),
                (T::RGBA32_128x128, 128),
                (T::RGBA32_128x128_2x, 256),
                (T::RGBA32_256x256, 256),
                (T::RGBA32_256x256_2x, 512),
                (T::RGBA32_512x512, 512),
                (T::RGBA32_512x512_2x, 1024),
            ] {
                let im = sized(s);
                let icon = icns::Image::from_data(icns::PixelFormat::RGBA, s, s, im.into_raw())?;
                fam.add_icon_with_type(&icon, ty)?;
            }
            let mut b = Vec::new();
            fam.write(&mut b)?;
            common::write(&dir.join("icon.icns"), &b)?;
            count += 1;
        }
        cx.progress(0.5);
        // Web: favicons, apple-touch-icon (opaque), PWA icons + manifest.
        if all || t == "web" {
            for s in [16u32, 32] {
                png(&dir.join(format!("favicon-{s}x{s}.png")), &sized(s))?;
            }
            png(&dir.join("apple-touch-icon.png"), &flatten(&sized(180), opaque_bg))?;
            png(&dir.join("android-chrome-192x192.png"), &sized(192))?;
            png(&dir.join("android-chrome-512x512.png"), &sized(512))?;
            let name = if v.str("app_name").trim().is_empty() { input.stem() } else { v.str("app_name").trim().to_string() };
            let theme = v.str("theme_color");
            let manifest = serde_json::json!({
                "name": name, "short_name": name,
                "icons": [
                    {"src": "/android-chrome-192x192.png", "sizes": "192x192", "type": "image/png"},
                    {"src": "/android-chrome-512x512.png", "sizes": "512x512", "type": "image/png"}
                ],
                "theme_color": &theme[..theme.len().min(7)], "background_color": &theme[..theme.len().min(7)], "display": "standalone"
            });
            common::write(&dir.join("site.webmanifest"), serde_json::to_string_pretty(&manifest)?.as_bytes())?;
            let snippet = "<link rel=\"icon\" href=\"/favicon.ico\" sizes=\"any\">\n<link rel=\"icon\" type=\"image/png\" sizes=\"32x32\" href=\"/favicon-32x32.png\">\n<link rel=\"icon\" type=\"image/png\" sizes=\"16x16\" href=\"/favicon-16x16.png\">\n<link rel=\"apple-touch-icon\" sizes=\"180x180\" href=\"/apple-touch-icon.png\">\n<link rel=\"manifest\" href=\"/site.webmanifest\">\n";
            common::write(&dir.join("favicon.html"), snippet.as_bytes())?;
            count += 7;
        }
        cx.progress(0.7);
        // iOS: AppIcon.appiconset (opaque, no transparency allowed by the App Store).
        if all || t == "ios" {
            let set = dir.join("AppIcon.appiconset");
            let entries: &[(&str, &str, u32, &str)] = &[
                ("iphone", "20x20", 2, "Icon-20@2x.png"),
                ("iphone", "20x20", 3, "Icon-20@3x.png"),
                ("iphone", "29x29", 2, "Icon-29@2x.png"),
                ("iphone", "29x29", 3, "Icon-29@3x.png"),
                ("iphone", "40x40", 2, "Icon-40@2x.png"),
                ("iphone", "40x40", 3, "Icon-40@3x.png"),
                ("iphone", "60x60", 2, "Icon-60@2x.png"),
                ("iphone", "60x60", 3, "Icon-60@3x.png"),
                ("ipad", "20x20", 1, "Icon-20.png"),
                ("ipad", "20x20", 2, "Icon-20@2x-ipad.png"),
                ("ipad", "29x29", 1, "Icon-29.png"),
                ("ipad", "29x29", 2, "Icon-29@2x-ipad.png"),
                ("ipad", "40x40", 1, "Icon-40.png"),
                ("ipad", "40x40", 2, "Icon-40@2x-ipad.png"),
                ("ipad", "76x76", 1, "Icon-76.png"),
                ("ipad", "76x76", 2, "Icon-76@2x.png"),
                ("ipad", "83.5x83.5", 2, "Icon-83.5@2x.png"),
                ("ios-marketing", "1024x1024", 1, "Icon-1024.png"),
            ];
            let mut images = Vec::new();
            for (idiom, size, scale, file) in entries {
                let pts: f32 = size.split('x').next().and_then(|s| s.parse().ok()).unwrap_or(20.0);
                let px = (pts * *scale as f32).round() as u32;
                png(&set.join(file), &flatten(&sized(px), opaque_bg))?;
                images.push(serde_json::json!({"idiom": idiom, "size": size, "scale": format!("{scale}x"), "filename": file}));
            }
            let contents = serde_json::json!({"images": images, "info": {"version": 1, "author": "Castor"}});
            common::write(&set.join("Contents.json"), serde_json::to_string_pretty(&contents)?.as_bytes())?;
            count += entries.len() + 1;
        }
        cx.progress(0.85);
        // Android: legacy launcher icons per density + Play Store icon.
        if all || t == "android" {
            for (dens, s) in [("mdpi", 48u32), ("hdpi", 72), ("xhdpi", 96), ("xxhdpi", 144), ("xxxhdpi", 192)] {
                png(&dir.join("android").join(format!("mipmap-{dens}")).join("ic_launcher.png"), &sized(s))?;
            }
            png(&dir.join("android").join("playstore-icon.png"), &flatten(&sized(512), opaque_bg))?;
            count += 6;
        }
        cx.progress(1.0);
        Ok(vec![Output::new(dir).with_note(castor_core::tr_args("image.icon-pack.files", &[("count", &count.to_string())]))])
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let Some(f) = input else { return Ok(None) };
        let img = common::load_preview(f, 512)?.to_rgba8();
        let m = master(&img, v, 512);
        // A strip of the main sizes on a checkerboard.
        let sizes = [16u32, 32, 64, 128, 256];
        let w: u32 = sizes.iter().sum::<u32>() + 20 * (sizes.len() as u32 + 1);
        let mut strip = RgbaImage::from_pixel(w, 296, Rgba([0, 0, 0, 0]));
        let mut x = 20;
        for s in sizes {
            let im = ops::resize_rgba(&m, s, s);
            ops::overlay(&mut strip, &im, x as i64, (276 - s) as i64, 1.0);
            x += s + 20;
        }
        common::preview(&crate::ml_tools::checkerboard_under(&strip), max_side, None)
    }
}

/// Square master: padded, on the background, with optional rounded corners.
fn master(img: &RgbaImage, v: &Values, side: u32) -> RgbaImage {
    let pad = (side as f32 * v.int("padding").clamp(0, 45) as f32 / 100.0) as u32;
    let inner = side - 2 * pad;
    let fitted = ops::fit_square(img, inner, [0, 0, 0, 0]);
    let mut m = RgbaImage::from_pixel(side, side, Rgba(v.color("background")));
    ops::overlay(&mut m, &fitted, pad as i64, pad as i64, 1.0);
    let r = side as f32 * v.int("radius").clamp(0, 50) as f32 / 100.0;
    ops::round_corners(&mut m, r);
    m
}

fn flatten(img: &RgbaImage, bg: [u8; 4]) -> RgbaImage {
    let mut out = RgbaImage::from_pixel(img.width(), img.height(), Rgba([bg[0], bg[1], bg[2], 255]));
    ops::overlay(&mut out, img, 0, 0, 1.0);
    out
}
