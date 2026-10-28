//! RGB to NV12 on the GPU's video processor (VAAPI `VAEntrypointVideoProc`), for the zero-copy path (docs/GPU-PLAN.md, stage 3): the captured
//! picture stays in GPU memory, and so does its conversion to the encoder's input. Same conversion as the CPU one in `encode_va.rs`: full range,
//! BT.709. The picture goes to the top left corner of the (larger) NV12 surface and the rest is black.

use std::ffi::c_void;
use std::rc::Rc;

use anyhow::{anyhow, bail, Result};
use cros_codecs::libva::{self, Config, Context, Display, Surface, SurfaceMemoryDescriptor, VAEntrypoint, VAProfile};

/// How the video processor converts: what its quality options are set to (see the test below for what was measured).
#[derive(Clone, Copy, Debug)]
pub struct Quality {
    pub filter_flags: u32,
    pub pipeline_flags: u32,
    pub input_siting: u8,
    pub output_siting: u8,
}

/// `VA_CHROMA_SITING_VERTICAL_CENTER | VA_CHROMA_SITING_HORIZONTAL_CENTER`.
const SITING_CENTRE: u8 = 2 | 8;

impl Quality {
    /// The colour of each pair of pixels is taken at their centre, as the CPU conversion does (the average of the four). The processor's own default
    /// puts it at the left edge of the pair: half a chroma pixel off, and thin light strokes on dark (text) came out with coloured fringes (chroma
    /// error 5.2 against the CPU's, 0.08 with this). The scaling quality flags make no difference to this conversion (measured).
    pub const DEFAULT: Quality = Quality { filter_flags: 0, pipeline_flags: 0, input_siting: 0, output_siting: SITING_CENTRE };
}

pub struct Vpp {
    display: Rc<Display>,
    _config: Config,
    context: Rc<Context>,
}

fn check(status: libva::VAStatus, what: &str) -> Result<()> {
    if status == libva::VA_STATUS_SUCCESS as libva::VAStatus {
        Ok(())
    } else {
        bail!("VA {what} failed ({status})")
    }
}

impl Vpp {
    /// A video processor for surfaces up to `width` x `height`.
    pub fn new(display: &Rc<Display>, width: u32, height: u32) -> Result<Self> {
        let config = display
            .create_config(vec![], VAProfile::VAProfileNone, VAEntrypoint::VAEntrypointVideoProc)
            .map_err(|e| anyhow!("VA video processing config: {e}"))?;
        let context = display.create_context::<()>(&config, width, height, None, true).map_err(|e| anyhow!("VA video processing context: {e}"))?;
        Ok(Self { display: Rc::clone(display), _config: config, context })
    }

    /// Converts the top left `w` x `h` of `src` (an RGB32 surface) into `dst` (NV12) and waits for it; `flip`: the source is upside down.
    pub fn convert<S: SurfaceMemoryDescriptor, D: SurfaceMemoryDescriptor>(&self, src: &Surface<S>, dst: &Surface<D>, w: u32, h: u32, flip: bool) -> Result<()> {
        self.convert_with(src, dst, w, h, flip, Quality::DEFAULT)
    }

    pub fn convert_with<S: SurfaceMemoryDescriptor, D: SurfaceMemoryDescriptor>(&self, src: &Surface<S>, dst: &Surface<D>, w: u32, h: u32, flip: bool, q: Quality) -> Result<()> {
        let region = libva::VARectangle { x: 0, y: 0, width: w as u16, height: h as u16 };
        // SAFETY: the structure is plain data; `region` outlives the calls that read it; every VA object belongs to `self.display`.
        unsafe {
            let mut p: libva::VAProcPipelineParameterBuffer = std::mem::zeroed();
            p.surface = src.id();
            p.surface_region = &region;
            p.surface_color_standard = libva::_VAProcColorStandardType_VAProcColorStandardNone;
            p.output_region = &region;
            p.output_background_color = 0xff00_0000;
            p.mirror_state = if flip { libva::VA_MIRROR_VERTICAL } else { libva::VA_MIRROR_NONE };
            p.output_color_standard = libva::_VAProcColorStandardType_VAProcColorStandardBT709;
            p.input_color_properties.color_range = libva::VA_SOURCE_RANGE_FULL as u8;
            p.output_color_properties.color_range = libva::VA_SOURCE_RANGE_FULL as u8;
            p.filter_flags = q.filter_flags;
            p.output_color_properties.chroma_sample_location = q.output_siting;
            p.input_color_properties.chroma_sample_location = q.input_siting;
            p.pipeline_flags = q.pipeline_flags;
            let (dpy, ctx) = (self.display.handle(), self.context.id());
            let mut buffer: libva::VABufferID = 0;
            check(
                libva::vaCreateBuffer(
                    dpy,
                    ctx,
                    libva::VABufferType::VAProcPipelineParameterBufferType,
                    std::mem::size_of::<libva::VAProcPipelineParameterBuffer>() as u32,
                    1,
                    (&raw mut p).cast::<c_void>(),
                    &mut buffer,
                ),
                "vaCreateBuffer",
            )?;
            let result = (|| {
                check(libva::vaBeginPicture(dpy, ctx, dst.id()), "vaBeginPicture")?;
                check(libva::vaRenderPicture(dpy, ctx, &mut buffer, 1), "vaRenderPicture")?;
                check(libva::vaEndPicture(dpy, ctx), "vaEndPicture")
            })();
            libva::vaDestroyBuffer(dpy, buffer);
            result?;
        }
        dst.sync().map_err(|e| anyhow!("VA sync: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cros_codecs::libva::{UsageHint, VA_FOURCC_BGRX, VA_FOURCC_NV12, VA_RT_FORMAT_RGB32, VA_RT_FORMAT_YUV420};
    use yuv::{bgra_to_yuv_nv12, BufferStoreMut, YuvBiPlanarImageMut, YuvConversionMode, YuvRange, YuvStandardMatrix};

    fn image_format(display: &Display, fourcc: u32) -> libva::VAImageFormat {
        display.query_image_formats().unwrap().into_iter().find(|f| f.fourcc == fourcc).expect("image format")
    }

    /// Needs a GPU with a video processor: `cargo test vpp -- --ignored` on such a machine.
    #[test]
    #[ignore]
    fn gpu_conversion_matches_the_cpu_one() {
        let display = Display::open_drm_display("/dev/dri/renderD128").expect("VA display");
        let (w, h, sw, sh) = (250usize, 122usize, 272u32, 144u32);
        // A picture with smooth gradients, a few hard edges and some colour: what a desktop is made of.
        let mut bgra = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                let edge = if (x / 16 + y / 16) % 7 == 0 { 255 } else { 0 };
                bgra[i] = ((x * 255) / w) as u8 ^ edge;
                bgra[i + 1] = ((y * 255) / h) as u8;
                bgra[i + 2] = (((x + y) * 255) / (w + h)) as u8 ^ edge;
                bgra[i + 3] = 255;
            }
        }
        let rgb = display.create_surfaces(VA_RT_FORMAT_RGB32, Some(VA_FOURCC_BGRX), sw, sh, Some(UsageHint::USAGE_HINT_ENCODER), vec![()]).unwrap().pop().unwrap();
        {
            let mut image = libva::Image::create_from(&rgb, image_format(&display, VA_FOURCC_BGRX), (sw, sh), (sw, sh)).unwrap();
            let (off, pitch) = (image.image().offsets[0] as usize, image.image().pitches[0] as usize);
            let data = image.as_mut();
            for y in 0..sh as usize {
                let row = &mut data[off + y * pitch..off + y * pitch + sw as usize * 4];
                if y < h {
                    row[..w * 4].copy_from_slice(&bgra[y * w * 4..(y + 1) * w * 4]);
                    row[w * 4..].fill(0x7f);
                } else {
                    row.fill(0x7f);
                }
            }
        }
        let nv12 = display.create_surfaces(VA_RT_FORMAT_YUV420, Some(VA_FOURCC_NV12), sw, sh, Some(UsageHint::USAGE_HINT_ENCODER), vec![()]).unwrap().pop().unwrap();
        Vpp::new(&display, sw, sh).unwrap().convert(&rgb, &nv12, w as u32, h as u32, false).unwrap();

        // CPU reference.
        let (mut cy, mut cuv) = (vec![0u8; w * h], vec![0u8; w * h.div_ceil(2)]);
        let mut img = YuvBiPlanarImageMut {
            y_plane: BufferStoreMut::Borrowed(&mut cy),
            y_stride: w as u32,
            uv_plane: BufferStoreMut::Borrowed(&mut cuv),
            uv_stride: w as u32,
            width: w as u32,
            height: h as u32,
        };
        bgra_to_yuv_nv12(&mut img, &bgra, (w * 4) as u32, YuvRange::Full, YuvStandardMatrix::Bt709, YuvConversionMode::Balanced).unwrap();

        let image = libva::Image::create_from(&nv12, image_format(&display, VA_FOURCC_NV12), (sw, sh), (sw, sh)).unwrap();
        let va = *image.image();
        let data = image.as_ref();
        let (mut sum, mut max, mut n) = (0u64, 0i32, 0u64);
        for y in 0..h {
            for x in 0..w {
                let g = i32::from(data[va.offsets[0] as usize + y * va.pitches[0] as usize + x]);
                let c = i32::from(cy[y * w + x]);
                sum += (g - c).unsigned_abs() as u64;
                max = max.max((g - c).abs());
                n += 1;
            }
        }
        let (luma_mean, luma_max) = (sum as f64 / n as f64, max);
        let (mut sum, mut max, mut n) = (0u64, 0i32, 0u64);
        for y in 0..h / 2 {
            for x in 0..w {
                let g = i32::from(data[va.offsets[1] as usize + y * va.pitches[1] as usize + x]);
                let c = i32::from(cuv[y * w + x]);
                sum += (g - c).unsigned_abs() as u64;
                max = max.max((g - c).abs());
                n += 1;
            }
        }
        let (chroma_mean, chroma_max) = (sum as f64 / n as f64, max);
        // The padding is black: luma 0 (full range), chroma 128.
        let pad_luma = data[va.offsets[0] as usize + (sh as usize - 1) * va.pitches[0] as usize + sw as usize - 1];
        let pad_chroma = data[va.offsets[1] as usize + (sh as usize / 2 - 1) * va.pitches[1] as usize + sw as usize - 2];
        println!("luma: mean |diff| {luma_mean:.2}, max {luma_max}; chroma: mean |diff| {chroma_mean:.2}, max {chroma_max}; padding Y {pad_luma} UV {pad_chroma}");
        assert!(luma_mean < 1.5 && chroma_mean < 2.0, "the GPU's colours differ from the CPU's");
    }

    /// Chroma on text-like content for the video processor's quality options, against the CPU's box-filtered chroma. Prints a table.
    #[test]
    #[ignore]
    fn chroma_quality_options() {
        let display = Display::open_drm_display("/dev/dri/renderD128").expect("VA display");
        let (w, h, sw, sh) = (400usize, 120usize, 416u32, 144u32);
        // Light thin strokes on a dark background, in a few colours: what the top bar and a terminal are made of.
        let mut bgra = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                let stroke = (x % 7 < 2) ^ ((y / 12) % 2 == 0 && x % 5 == 0);
                let (r, g, b) = if stroke {
                    match (x / 40) % 3 {
                        0 => (235, 235, 245),
                        1 => (120, 200, 255),
                        _ => (255, 190, 90),
                    }
                } else {
                    (14, 12, 30)
                };
                bgra[i..i + 4].copy_from_slice(&[b, g, r, 255]);
            }
        }
        let rgb = display.create_surfaces(VA_RT_FORMAT_RGB32, Some(VA_FOURCC_BGRX), sw, sh, Some(UsageHint::USAGE_HINT_ENCODER), vec![()]).unwrap().pop().unwrap();
        {
            let mut image = libva::Image::create_from(&rgb, image_format(&display, VA_FOURCC_BGRX), (sw, sh), (sw, sh)).unwrap();
            let (off, pitch) = (image.image().offsets[0] as usize, image.image().pitches[0] as usize);
            let data = image.as_mut();
            for y in 0..h {
                data[off + y * pitch..off + y * pitch + w * 4].copy_from_slice(&bgra[y * w * 4..(y + 1) * w * 4]);
            }
        }
        let (mut cy, mut cuv) = (vec![0u8; w * h], vec![0u8; w * h.div_ceil(2)]);
        let mut img = YuvBiPlanarImageMut {
            y_plane: BufferStoreMut::Borrowed(&mut cy),
            y_stride: w as u32,
            uv_plane: BufferStoreMut::Borrowed(&mut cuv),
            uv_stride: w as u32,
            width: w as u32,
            height: h as u32,
        };
        bgra_to_yuv_nv12(&mut img, &bgra, (w * 4) as u32, YuvRange::Full, YuvStandardMatrix::Bt709, YuvConversionMode::Balanced).unwrap();
        let vpp = Vpp::new(&display, sw, sh).unwrap();
        let variants: [(&str, Quality); 9] = [
            ("default (centre siting)", Quality::DEFAULT),
            ("old default (left siting)", Quality { output_siting: 0, ..Quality::DEFAULT }),
            ("scaling FAST", Quality { filter_flags: libva::VA_FILTER_SCALING_FAST, ..Quality::DEFAULT }),
            ("scaling HQ", Quality { filter_flags: libva::VA_FILTER_SCALING_HQ, ..Quality::DEFAULT }),
            ("scaling NL anamorphic", Quality { filter_flags: libva::VA_FILTER_SCALING_NL_ANAMORPHIC, ..Quality::DEFAULT }),
            ("HQ + siting left/centre", Quality { filter_flags: libva::VA_FILTER_SCALING_HQ, output_siting: 4 | 2, input_siting: 0, pipeline_flags: 0 }),
            ("default + siting left/centre", Quality { output_siting: 4 | 2, ..Quality::DEFAULT }),
            ("HQ + siting centre/centre", Quality { filter_flags: libva::VA_FILTER_SCALING_HQ, output_siting: 8 | 2, ..Quality::DEFAULT }),
            ("HQ + pipeline FAST", Quality { filter_flags: libva::VA_FILTER_SCALING_HQ, pipeline_flags: libva::VA_PROC_PIPELINE_FAST, ..Quality::DEFAULT }),
        ];
        for (name, q) in variants {
            let nv12 = display.create_surfaces(VA_RT_FORMAT_YUV420, Some(VA_FOURCC_NV12), sw, sh, Some(UsageHint::USAGE_HINT_ENCODER), vec![()]).unwrap().pop().unwrap();
            if let Err(e) = vpp.convert_with(&rgb, &nv12, w as u32, h as u32, false, q) {
                println!("{name:32} failed: {e}");
                continue;
            }
            let image = libva::Image::create_from(&nv12, image_format(&display, VA_FOURCC_NV12), (sw, sh), (sw, sh)).unwrap();
            let va = *image.image();
            let data = image.as_ref();
            let (mut sy, mut sc, mut ny, mut nc) = (0u64, 0u64, 0u64, 0u64);
            for y in 0..h {
                for x in 0..w {
                    let g = i32::from(data[va.offsets[0] as usize + y * va.pitches[0] as usize + x]);
                    sy += (g - i32::from(cy[y * w + x])).unsigned_abs() as u64;
                    ny += 1;
                }
            }
            for y in 0..h / 2 {
                for x in 0..w {
                    let g = i32::from(data[va.offsets[1] as usize + y * va.pitches[1] as usize + x]);
                    sc += (g - i32::from(cuv[y * w + x])).unsigned_abs() as u64;
                    nc += 1;
                }
            }
            println!("{name:32} luma |diff| {:5.2}   chroma |diff| {:5.2}", sy as f64 / ny as f64, sc as f64 / nc as f64);
        }
    }
}
