//! H.264 on NVIDIA's fixed-function encoder (NVENC), for sessions on a machine with an NVIDIA card (docs/GPU-PLAN.md).
//! Same contract as `encode::H264`: a BGRA frame in, an Annex B access unit out (SPS and PPS at every keyframe), a bitrate that can change
//! while it runs. NVENC takes the BGRA picture as it is (no colour conversion on our side): it goes to the card through a CUDA buffer.
//!
//! Like the VAAPI encoder it runs at a constant quantiser, which `QpControl` moves with the link: the quantiser is changed in the running
//! encoder (`nvEncReconfigureEncoder`, no keyframe, no new parameter sets), because a client that meets new parameter sets in a row shows a
//! washed-out picture (Windows App). The CUDA and NVENC libraries are loaded when the encoder is created, so a machine without NVIDIA's
//! driver is not affected by having this in the binary.

use std::ffi::c_void;
use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use cudarc::driver::{result, sys, CudaContext};
use nvidia_video_codec_sdk::sys::nvEncodeAPI::{
    NVENCSTATUS, NV_ENC_BUFFER_FORMAT, NV_ENC_CODEC_H264_GUID, NV_ENC_CONFIG, NV_ENC_H264_PROFILE_HIGH_GUID, NV_ENC_INITIALIZE_PARAMS,
    NV_ENC_INITIALIZE_PARAMS_VER, NV_ENC_INPUT_RESOURCE_TYPE, NV_ENC_PARAMS_RC_MODE, NV_ENC_PRESET_P1_GUID, NV_ENC_QP, NV_ENC_RECONFIGURE_PARAMS,
    NV_ENC_RECONFIGURE_PARAMS_VER, NV_ENC_TUNING_INFO, NVENC_INFINITE_GOPLENGTH,
};
use nvidia_video_codec_sdk::{Bitstream, EncodePictureParams, Encoder, EncoderInitParams, RegisteredResource, Session, ENCODE_API};

use crate::encode::{surface_size, Tuning};
use crate::encoder::Encode;
use crate::qp::QpControl;

/// `NV_ENC_PIC_FLAG_FORCEINTRA`: an intra picture (a refresh of what moved) that is not an IDR, so it brings no new SPS and PPS. That is what the
/// settle refresh of the worker asks for, as with the VAAPI encoder; the first picture of an encoder is an IDR with its parameter sets by itself.
const FORCE_INTRA: u32 = 1;

pub struct NvH264 {
    // What borrows the session comes first: struct fields drop in order, and these must go before the session they point into.
    resource: RegisteredResource<'static, ()>,
    bitstream: Bitstream<'static>,
    session: Box<Session>,
    /// The picture on the card: `surface` wide and high, 4 bytes a pixel, what `resource` is registered on.
    dev: DeviceBuffer,
    ctx: Arc<CudaContext>,
    /// The picture in host memory with its padding (black) up to the surface size, the rows of a frame copied into it.
    stage: Vec<u8>,
    surface: (usize, usize),
    config: Box<NV_ENC_CONFIG>,
    framerate: u32,
    qp: QpControl,
    current_qp: u32,
    refresh: bool,
    timestamp: u64,
}

// SAFETY: the raw pointers inside (the NVENC session, its buffers) are used from one thread at a time: the encoder is built on the thread
// that moves it to the capture loop and never shared, and every call binds the CUDA context to the calling thread first.
unsafe impl Send for NvH264 {}

/// Plain `cuMemAlloc` memory. cudarc's safe allocation uses the stream-ordered pool where the device has one, and NVENC does not take pool memory
/// as an input resource (the registration fails).
struct DeviceBuffer {
    ptr: sys::CUdeviceptr,
    ctx: Arc<CudaContext>,
}

impl DeviceBuffer {
    fn new(ctx: &Arc<CudaContext>, len: usize) -> Result<Self> {
        ctx.bind_to_thread().map_err(|e| anyhow!("CUDA: {e}"))?;
        // SAFETY: the context is current on this thread; the buffer is freed in `drop` with the same context.
        let ptr = unsafe { result::malloc_sync(len) }.map_err(|e| anyhow!("CUDA buffer: {e}"))?;
        // SAFETY: `ptr` was just allocated with `len` bytes.
        unsafe { result::memset_d8_sync(ptr, 0, len) }.map_err(|e| anyhow!("CUDA buffer: {e}"))?;
        Ok(Self { ptr, ctx: Arc::clone(ctx) })
    }

    /// Copies `src` to the start of the buffer, which it must not be longer than.
    fn upload(&self, src: &[u8]) -> Result<()> {
        // SAFETY: the context is current (the caller bound it) and the buffer holds the surface the encoder was made for.
        unsafe { result::memcpy_htod_sync(self.ptr, src) }.map_err(|e| anyhow!("CUDA copy: {e}"))
    }
}

impl Drop for DeviceBuffer {
    fn drop(&mut self) {
        let _ = self.ctx.bind_to_thread();
        // SAFETY: `ptr` is ours, from `malloc_sync`, freed once.
        let _ = unsafe { result::free_sync(self.ptr) };
    }
}

/// `0000:01:00.0`: the PCI address of a CUDA device, the way sysfs names it.
fn pci_string(domain: i32, bus: i32, device: i32) -> String {
    format!("{domain:04x}:{bus:02x}:{device:02x}.0")
}

/// The PCI address of the card behind a render node (`/dev/dri/renderD129`), from sysfs.
fn node_pci(node: &Path) -> Option<String> {
    let name = node.file_name()?.to_str()?;
    let device = std::fs::canonicalize(format!("/sys/class/drm/{name}/device")).ok()?;
    Some(device.file_name()?.to_string_lossy().to_lowercase())
}

/// The CUDA ordinal whose PCI address is `pci`.
fn device_for(pci: &str, cuda: &[String]) -> Option<usize> {
    cuda.iter().position(|d| d.eq_ignore_ascii_case(pci))
}

/// The PCI address of every CUDA device, by ordinal (no context is made for them).
fn cuda_pci_addresses() -> Result<Vec<String>> {
    result::init().map_err(|e| anyhow!("CUDA: {e}"))?;
    let count = result::device::get_count().map_err(|e| anyhow!("CUDA: {e}"))?;
    (0..count)
        .map(|i| {
            let dev = result::device::get(i).map_err(|e| anyhow!("CUDA: {e}"))?;
            let attr = |a: sys::CUdevice_attribute| {
                // SAFETY: `dev` is a device CUDA just returned.
                unsafe { result::device::get_attribute(dev, a) }.map_err(|e| anyhow!("CUDA: {e}"))
            };
            Ok(pci_string(
                attr(sys::CUdevice_attribute::CU_DEVICE_ATTRIBUTE_PCI_DOMAIN_ID)?,
                attr(sys::CUdevice_attribute::CU_DEVICE_ATTRIBUTE_PCI_BUS_ID)?,
                attr(sys::CUdevice_attribute::CU_DEVICE_ATTRIBUTE_PCI_DEVICE_ID)?,
            ))
        })
        .collect()
}

/// The CUDA device to encode on: `WAYDESK_NVENC_DEVICE` when it says one; else the card the session renders on, if that is an NVIDIA card; else
/// the first (a machine with an Intel or AMD card that renders and an NVIDIA one that only encodes, or one NVIDIA card).
fn choose_device(node: Option<&Path>) -> usize {
    if let Some(n) = std::env::var("WAYDESK_NVENC_DEVICE").ok().and_then(|v| v.trim().parse::<usize>().ok()) {
        return n;
    }
    let Some(pci) = node.and_then(node_pci) else { return 0 };
    match cuda_pci_addresses() {
        Ok(list) => device_for(&pci, &list).unwrap_or(0),
        Err(_) => 0,
    }
}

fn qp_of(qp: u32) -> NV_ENC_QP {
    let q = qp.clamp(1, 51);
    NV_ENC_QP { qpInterP: q, qpInterB: q, qpIntra: q }
}

impl NvH264 {
    /// `node` is the render node the session's sway draws on, if any: the encoder then runs on that card when it is an NVIDIA one.
    pub fn new(node: Option<&Path>, width: u16, height: u16, t: Tuning) -> Result<Self> {
        let (sw, sh) = surface_size(usize::from(width), usize::from(height));
        let ordinal = choose_device(node);
        tracing::debug!(device = ordinal, "NVENC on this CUDA device");
        let ctx = CudaContext::new(ordinal).map_err(|e| anyhow!("CUDA device {ordinal}: {e}"))?;
        let encoder = Encoder::initialize_with_cuda(Arc::clone(&ctx)).map_err(|e| anyhow!("NVENC: {e:?}"))?;
        let encodes_h264 = encoder.get_encode_guids().map_err(|e| anyhow!("NVENC: {e:?}"))?.contains(&NV_ENC_CODEC_H264_GUID);
        if !encodes_h264 {
            bail!("this NVIDIA GPU has no H.264 encoder");
        }
        // Floor: the quality wanted (as for the VAAPI encoder); ceiling: what a slow link may cost. `WAYDESK_VA_MIN_QP` and `WAYDESK_VA_MAX_QP` apply here too.
        let env_qp = |name: &str| std::env::var(name).ok().and_then(|v| v.parse::<u32>().ok()).filter(|q| (1..=51).contains(q));
        let floor = env_qp("WAYDESK_VA_MIN_QP").unwrap_or_else(|| u32::from(t.max_qp.clamp(1, 51)).saturating_sub(2).max(1));
        let ceiling = env_qp("WAYDESK_VA_MAX_QP").unwrap_or_else(|| floor.max(40));

        let tuning = NV_ENC_TUNING_INFO::NV_ENC_TUNING_INFO_ULTRA_LOW_LATENCY;
        let preset = encoder.get_preset_config(NV_ENC_CODEC_H264_GUID, NV_ENC_PRESET_P1_GUID, tuning).map_err(|e| anyhow!("NVENC preset: {e:?}"))?;
        let mut config = Box::new(preset.presetCfg);
        config.profileGUID = NV_ENC_H264_PROFILE_HIGH_GUID;
        config.gopLength = NVENC_INFINITE_GOPLENGTH;
        config.frameIntervalP = 1; // no B frames: a picture goes out as it comes in
        config.rcParams.rateControlMode = NV_ENC_PARAMS_RC_MODE::NV_ENC_PARAMS_RC_CONSTQP;
        config.rcParams.constQP = qp_of(floor);
        // SAFETY: `h264Config` is the member of the codec union that applies to an H.264 encode.
        unsafe {
            let h = &mut config.encodeCodecConfig.h264Config;
            h.idrPeriod = NVENC_INFINITE_GOPLENGTH;
            h.maxNumRefFrames = 1;
            h.chromaFormatIDC = 1;
            h.set_repeatSPSPPS(1);
        }
        let mut params = EncoderInitParams::new(NV_ENC_CODEC_H264_GUID, sw as u32, sh as u32);
        params.preset_guid(NV_ENC_PRESET_P1_GUID).tuning_info(tuning).encode_config(&mut config).framerate(t.fps.max(1), 1).enable_picture_type_decision();
        let session = Box::new(encoder.start_session(NV_ENC_BUFFER_FORMAT::NV_ENC_BUFFER_FORMAT_ARGB, params).map_err(|e| anyhow!("NVENC session: {e:?}"))?);

        let dev = DeviceBuffer::new(&ctx, sw * sh * 4)?;
        let ptr = dev.ptr as usize as *mut c_void;
        // SAFETY: the session is in a `Box` that this struct keeps (and drops last), so the extended lifetime of what borrows it holds.
        let session_ref: &'static Session = unsafe { &*(&*session as *const Session) };
        let resource = session_ref
            .register_generic_resource((), NV_ENC_INPUT_RESOURCE_TYPE::NV_ENC_INPUT_RESOURCE_TYPE_CUDADEVICEPTR, ptr, (sw * 4) as u32)
            .map_err(|e| anyhow!("NVENC input: {e:?}"))?;
        let bitstream = session_ref.create_output_bitstream().map_err(|e| anyhow!("NVENC output: {e:?}"))?;

        Ok(Self {
            resource,
            bitstream,
            session,
            dev,
            ctx,
            stage: vec![0; sw * sh * 4],
            surface: (sw, sh),
            config,
            framerate: t.fps.max(1),
            qp: QpControl::new(floor, ceiling, t.bitrate),
            current_qp: floor,
            refresh: false,
            timestamp: 0,
        })
    }

    /// Moves the running encoder to another quantiser without a keyframe or new parameter sets.
    fn reconfigure(&mut self, qp: u32) -> Result<()> {
        self.config.rcParams.constQP = qp_of(qp);
        let init = NV_ENC_INITIALIZE_PARAMS {
            version: NV_ENC_INITIALIZE_PARAMS_VER,
            encodeGUID: NV_ENC_CODEC_H264_GUID,
            presetGUID: NV_ENC_PRESET_P1_GUID,
            tuningInfo: NV_ENC_TUNING_INFO::NV_ENC_TUNING_INFO_ULTRA_LOW_LATENCY,
            encodeWidth: self.surface.0 as u32,
            encodeHeight: self.surface.1 as u32,
            darWidth: self.surface.0 as u32,
            darHeight: self.surface.1 as u32,
            frameRateNum: self.framerate,
            frameRateDen: 1,
            enablePTD: 1,
            encodeConfig: &mut *self.config,
            ..Default::default()
        };
        let mut params = NV_ENC_RECONFIGURE_PARAMS { version: NV_ENC_RECONFIGURE_PARAMS_VER, reInitEncodeParams: init, ..Default::default() };
        // SAFETY: the handle is the live session's, and `params` (with the config it points to) outlives the call.
        let status = unsafe { (ENCODE_API.reconfigure_encoder)(self.session.get_encoder().ptr(), &mut params) };
        if status != NVENCSTATUS::NV_ENC_SUCCESS {
            bail!("nvEncReconfigureEncoder: {status:?}");
        }
        Ok(())
    }
}

impl Encode for NvH264 {
    fn encode(&mut self, bgra: &[u8], w: usize, h: usize) -> Result<Option<Vec<u8>>> {
        let (sw, sh) = self.surface;
        if w > sw || h > sh || bgra.len() < w * h * 4 {
            bail!("a {w}x{h} frame for a {sw}x{sh} encoder");
        }
        self.ctx.bind_to_thread().map_err(|e| anyhow!("CUDA: {e}"))?;
        for (row, src) in bgra.chunks_exact(w * 4).take(h).enumerate() {
            self.stage[row * sw * 4..row * sw * 4 + w * 4].copy_from_slice(src);
        }
        self.dev.upload(&self.stage)?;

        let encode_flags = if std::mem::take(&mut self.refresh) { FORCE_INTRA } else { 0 };
        self.timestamp += 1;
        self.session
            .encode_picture(&mut self.resource, &mut self.bitstream, EncodePictureParams { input_timestamp: self.timestamp, encode_flags, ..Default::default() })
            .map_err(|e| anyhow!("NVENC encode: {e:?}"))?;
        let data = self.bitstream.lock().map_err(|e| anyhow!("NVENC output: {e:?}"))?.data().to_vec();

        let qp = self.qp.after_frame(data.len());
        if qp != self.current_qp {
            match self.reconfigure(qp) {
                Ok(()) => {
                    tracing::debug!(from = self.current_qp, to = qp, "quantiser");
                    self.current_qp = qp;
                }
                Err(e) => tracing::warn!(qp, "could not change the quantiser: {e:#}"),
            }
        }
        Ok(Some(data))
    }

    fn set_bitrate(&mut self, bps: u32) {
        self.qp.target_bps = f64::from(bps.max(1));
    }

    fn force_keyframe(&mut self) {
        self.refresh = true;
    }

    fn quantiser(&self) -> Option<u32> {
        Some(self.current_qp)
    }
}

/// The encoder, one frame at a time, on the real hardware: `cargo test nvenc -- --ignored` on a machine with an NVIDIA GPU.
#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context as _;

    fn frame(w: usize, h: usize, shift: usize) -> Vec<u8> {
        let mut v = vec![255u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                v[i] = ((x + shift) % 256) as u8; // B
                v[i + 1] = (y % 256) as u8; // G
                v[i + 2] = (((x + y) / 2) % 256) as u8; // R
            }
        }
        v
    }

    #[test]
    fn a_render_node_is_matched_to_its_cuda_device_by_pci_address() {
        let cuda = vec!["0000:01:00.0".to_string(), "0000:41:00.0".to_string()];
        assert_eq!(pci_string(0, 1, 0), "0000:01:00.0");
        assert_eq!(pci_string(0, 0x41, 0), "0000:41:00.0");
        assert_eq!(pci_string(0x10, 0xa, 0x1f), "0010:0a:1f.0");
        assert_eq!(device_for("0000:41:00.0", &cuda), Some(1));
        assert_eq!(device_for("0000:41:00.0".to_uppercase().as_str(), &cuda), Some(1), "sysfs and CUDA may differ in case");
        assert_eq!(device_for("0000:00:02.0", &cuda), None, "the Intel card of a hybrid machine is not a CUDA device");
        assert_eq!(node_pci(Path::new("/dev/dri/renderD999")), None, "a node that is not there has no address");
    }

    #[test]
    #[ignore = "needs an NVIDIA GPU"]
    fn nvenc_encodes_keyframes_and_inter_frames() {
        assert!(nvidia_video_codec_sdk::nvenc_available(), "libnvidia-encode.so.1 not found");
        let (w, h) = (1680usize, 1050usize);
        let mut enc = NvH264::new(None, w as u16, h as u16, Tuning { bitrate: 20_000_000, max_qp: 20, fps: 30 }).expect("encoder");
        let mut out = Vec::new();
        for i in 0..30 {
            let au = enc.encode(&frame(w, h, i * 3), w, h).expect("encode").expect("a frame");
            assert!(!au.is_empty(), "frame {i} is empty");
            if i == 0 {
                assert_eq!(&au[..4], &[0, 0, 0, 1], "Annex B start code");
                assert!(au.windows(5).any(|b| b[..4] == [0, 0, 0, 1] && b[4] & 0x1f == 7), "SPS in the first access unit");
                assert!(au.windows(5).any(|b| b[..4] == [0, 0, 0, 1] && b[4] & 0x1f == 5), "an IDR slice in the first access unit");
            }
            out.extend_from_slice(&au);
        }
        enc.force_keyframe();
        let au = enc.encode(&frame(w, h, 99), w, h).expect("encode").expect("a frame");
        assert!(!au.windows(5).any(|b| b[..4] == [0, 0, 0, 1] && matches!(b[4] & 0x1f, 7 | 8)), "a refresh brings no new parameter sets");
        out.extend_from_slice(&au);
        std::fs::write("/tmp/waydesk-nvenc-test.h264", &out).context("write").unwrap();
    }
}
