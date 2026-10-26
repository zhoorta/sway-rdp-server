//! `waydesk-encbench`: how much CPU the H.264 encoder takes, and how many bits it makes, for synthetic pictures. It runs the encoder of
//! the server with the settings in the environment (`WAYDESK_H264_USAGE`, `_COMPLEXITY`, `_THREADS`, `_SCENE_CHANGE`, `_BACKGROUND`,
//! `_ADAPTIVE_QUANT`, `_LTR`) and prints one line. It measures the encoder (the conversion to YUV included), not the capture.
//!
//!   waydesk-encbench [--scene pattern|video|desktop] [--frames N] [--size WxH] [--bitrate BITS] [--label TEXT] [--psnr] [--psnr-from N] [--qp N]
//!
//! With `--psnr` every frame is also decoded and compared with the source (the luma plane): the average PSNR in dB says how close the
//! picture is (higher is better; 40 dB and more is very hard to tell from the original).
//!
//! The scenes: `pattern` changes every pixel every frame (the worst case, nothing like a real desktop), `video` is a smooth picture with
//! moving coloured blocks, `settle` is the desktop with the scrolling stopped after 40 frames (the quality at rest: use `--psnr-from 80`), `desktop` is a window with text of which a third scrolls, and a blinking cursor (what most of a day looks like).

use std::time::Instant;

#[path = "../encode.rs"]
#[allow(dead_code)]
mod encode;

fn cpu_seconds() -> f64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: a valid timespec is passed.
    unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut ts) };
    ts.tv_sec as f64 + ts.tv_nsec as f64 / 1e9
}

fn hash(a: u32, b: u32) -> u32 {
    let mut h = a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^ (h >> 12)
}

fn put(frame: &mut [u8], w: usize, x: usize, y: usize, bgr: [u8; 3]) {
    let at = (y * w + x) * 4;
    frame[at..at + 4].copy_from_slice(&[bgr[0], bgr[1], bgr[2], 255]);
}

fn pattern(frame: &mut [u8], w: usize, h: usize, t: usize) {
    for y in 0..h {
        for x in 0..w {
            let b = ((x + 4 * t) ^ (y + 3 * t)) as u8;
            let g = (x * 255 / w + 5 * t) as u8;
            let r = (y * 255 / h) as u8;
            put(frame, w, x, y, [b, g, r]);
        }
    }
}

fn video(frame: &mut [u8], w: usize, h: usize, t: usize) {
    for y in 0..h {
        for x in 0..w {
            let g = ((x * 160 / w) + (y * 60 / h) + (t % 64)) as u8;
            put(frame, w, x, y, [g / 2 + 40, g, 200u8.saturating_sub(g / 2)]);
        }
    }
    for k in 0..6usize {
        let (bw, bh) = (w / 8, h / 6);
        let bx = (k * 211 + t * (3 + k)) % (w - bw);
        let by = (k * 97 + t * (2 + k % 3)) % (h - bh);
        let colour = [[230, 70, 60], [60, 200, 90], [70, 90, 230], [240, 220, 60], [200, 80, 200], [60, 200, 220]][k];
        for y in by..by + bh {
            for x in bx..bx + bw {
                put(frame, w, x, y, colour);
            }
        }
    }
}

fn desktop(frame: &mut [u8], w: usize, h: usize, t: usize) {
    // a window: dark title bar, a light page with lines of "text", a side bar; the text in the middle scrolls 3 px a frame
    for y in 0..h {
        for x in 0..w {
            let px = if y < 32 {
                [40, 36, 36]
            } else if x < 180 {
                [60, 55, 50]
            } else {
                let scrolling = x > w / 3 && x < w * 2 / 3 + 120 && y > 120 && y < h - 40;
                let row = if scrolling { y + 3 * t } else { y };
                let (line, inside) = (row / 16, row % 16);
                let word = hash(line as u32, (x / 9) as u32 + if scrolling { 7 } else { 0 });
                if inside > 2 && inside < 13 && (word & 3) != 0 && ((x % 9) < 5 + (word >> 8 & 3) as usize) {
                    [60, 60, 60]
                } else {
                    [245, 245, 245]
                }
            };
            put(frame, w, x, y, px);
        }
    }
    if (t / 15) % 2 == 0 {
        for y in 300..316 {
            for x in 420..422 {
                put(frame, w, x, y, [0, 0, 0]);
            }
        }
    }
}

fn main() {
    let mut scene = "pattern".to_string();
    let (mut frames, mut w, mut h, mut bitrate) = (300usize, 1280usize, 720usize, 10_000_000u32);
    let mut label = String::new();
    let mut want_psnr = false;
    let (mut psnr_from, mut max_qp) = (10usize, 28u8);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--scene" => scene = args.next().expect("--scene needs a name"),
            "--frames" => frames = args.next().and_then(|v| v.parse().ok()).expect("--frames needs a number"),
            "--bitrate" => bitrate = args.next().and_then(|v| v.parse().ok()).expect("--bitrate needs a number"),
            "--label" => label = args.next().expect("--label needs a text"),
            "--psnr" => want_psnr = true,
            "--psnr-from" => psnr_from = args.next().and_then(|v| v.parse().ok()).expect("--psnr-from needs a number"),
            "--qp" => max_qp = args.next().and_then(|v| v.parse().ok()).expect("--qp needs a number"),
            "--size" => {
                let v = args.next().expect("--size needs WxH");
                let (a, b) = v.split_once('x').expect("--size needs WxH");
                (w, h) = (a.parse().expect("width"), b.parse().expect("height"));
            }
            other => panic!("unknown option {other}"),
        }
    }
    let mut enc = encode::H264::new(w as u16, h as u16, encode::Tuning { bitrate, max_qp, fps: 30 }).expect("encoder");
    let mut frame = vec![0u8; w * h * 4];
    let (mut bytes, mut cpu, mut walls) = (0usize, 0f64, Vec::with_capacity(frames));
    let mut skipped = 0;
    let mut decoder = want_psnr.then(|| openh264::decoder::Decoder::with_api_config(encode::load_openh264().expect("openh264"), openh264::decoder::DecoderConfig::new()).expect("decoder"));
    let (mut psnr_sum, mut psnr_n) = (0f64, 0usize);
    let mut luma = vec![0u8; w * h];
    let mut u = vec![128u8; (w / 2) * (h / 2)];
    let mut v = vec![128u8; (w / 2) * (h / 2)];
    for t in 0..frames {
        match scene.as_str() {
            "pattern" => pattern(&mut frame, w, h, t),
            "video" => video(&mut frame, w, h, t),
            "desktop" => desktop(&mut frame, w, h, t),
            "settle" => desktop(&mut frame, w, h, t.min(40)),
            other => panic!("unknown scene {other}"),
        }
        let (c0, w0) = (cpu_seconds(), Instant::now());
        let out = enc.encode(&frame, w, h).expect("encode");
        cpu += cpu_seconds() - c0;
        walls.push(w0.elapsed().as_secs_f64() * 1000.0);
        match out {
            Some(data) => {
                bytes += data.len();
                if let Some(dec) = decoder.as_mut() {
                    if let Some(yuv) = dec.decode(&data).expect("decode") {
                        use openh264::formats::YUVSource as _;
                        // the source's luma, converted the way the encoder converts it
                        let mut img = yuv::YuvPlanarImageMut {
                            y_plane: yuv::BufferStoreMut::Borrowed(&mut luma),
                            y_stride: w as u32,
                            u_plane: yuv::BufferStoreMut::Borrowed(&mut u),
                            u_stride: (w / 2) as u32,
                            v_plane: yuv::BufferStoreMut::Borrowed(&mut v),
                            v_stride: (w / 2) as u32,
                            width: w as u32,
                            height: h as u32,
                        };
                        yuv::bgra_to_yuv420(&mut img, &frame, (w * 4) as u32, yuv::YuvRange::Full, yuv::YuvStandardMatrix::Bt709, yuv::YuvConversionMode::Balanced).expect("convert");
                        let (dy, stride) = (yuv.y(), yuv.strides().0);
                        let mut se = 0f64;
                        for row in 0..h {
                            for col in 0..w {
                                let d = f64::from(dy[row * stride + col]) - f64::from(luma[row * w + col]);
                                se += d * d;
                            }
                        }
                        if t >= psnr_from {
                            let mse = se / (w * h) as f64;
                            psnr_sum += if mse < 1e-9 { 100.0 } else { 10.0 * (255.0f64 * 255.0 / mse).log10() };
                            psnr_n += 1;
                        }
                    }
                }
            }
            None => skipped += 1,
        }
    }
    walls.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean_cpu_ms = cpu / frames as f64 * 1000.0;
    println!(
        "{label:34} {scene:8} {w}x{h}  {:6.2} Mbit/s at 30 fps  {:6.2} ms CPU/frame = {:5.1} % of a core at 30 fps  wall p50 {:5.1} ms p95 {:5.1} ms  skipped {skipped}  PSNR {}",
        bytes as f64 * 8.0 / (frames as f64 / 30.0) / 1e6,
        mean_cpu_ms,
        mean_cpu_ms * 30.0 / 10.0,
        walls[walls.len() / 2],
        walls[walls.len() * 95 / 100],
        if psnr_n > 0 { format!("{:.1} dB", psnr_sum / psnr_n as f64) } else { "-".into() },
    );
}
