//! The GPUs of the machine and how busy they are, for `waydesk-admin status`. What a session runs on is in `sessionstatus.rs`.
//!
//! The cards are found in `/sys/class/drm` (one per render node: vendor, device, driver, PCI address), the model from the PCI id database when the
//! system has one. How busy each is: NVIDIA through NVML (`libnvidia-ml`, loaded when a card is there, nothing is linked), AMD from `gpu_busy_percent`
//! in sysfs, Intel from the time the card spends out of its RC6 idle state (the share of a second it was awake; not the split between render and
//! video engines, which needs `perf` or `intel_gpu_top`). Nothing needs root, and nothing is installed for it.
#![allow(dead_code)]

use std::ffi::{c_char, c_int, c_uint, c_ulonglong, c_void, CString};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Util {
    /// Percent of the time the card was working.
    pub busy: Option<f64>,
    /// Percent of the time its video encoder was working (NVIDIA).
    pub encoder: Option<f64>,
    pub mem_used_mb: Option<u64>,
    pub mem_total_mb: Option<u64>,
    pub mhz: Option<u32>,
    pub watts: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gpu {
    /// `/dev/dri/renderD128`.
    pub node: String,
    /// `intel`, `nvidia`, `amd` or the PCI vendor id.
    pub vendor: String,
    pub model: String,
    /// The kernel driver: `i915`, `xe`, `nvidia`, `amdgpu`, `nouveau`.
    pub driver: String,
    /// `0000:01:00.0`.
    pub pci: String,
    /// The `cardN` directory of the same device.
    pub card: String,
    pub util: Util,
}

const DRM: &str = "/sys/class/drm";

fn read(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim().to_owned())
}

fn read_num(p: &Path) -> Option<u64> {
    read(p)?.parse().ok()
}

fn hex(p: &Path) -> Option<u32> {
    u32::from_str_radix(read(p)?.trim_start_matches("0x"), 16).ok()
}

pub fn vendor_name(id: u32) -> String {
    match id {
        0x8086 => "intel".into(),
        0x10de => "nvidia".into(),
        0x1002 | 0x1022 => "amd".into(),
        other => format!("{other:04x}"),
    }
}

/// The name a PCI id database gives a device (`pci.ids`), if the system has one: `(vendor, device)` as numbers.
pub fn pci_ids_lookup(text: &str, vendor: u32, device: u32) -> Option<String> {
    let want_v = format!("{vendor:04x}");
    let want_d = format!("{device:04x}");
    let mut in_vendor = false;
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if !line.starts_with('\t') {
            in_vendor = line.starts_with(&want_v) && line[4..].starts_with(char::is_whitespace);
            continue;
        }
        if in_vendor && !line.starts_with("\t\t") {
            let rest = line.trim_start_matches('\t');
            if rest.starts_with(&want_d) && rest[4..].starts_with(char::is_whitespace) {
                return Some(rest[4..].trim().to_owned());
            }
        }
    }
    None
}

/// "Intel AlderLake-S GT1": the database names a device without its maker, unless the name already has it.
pub fn with_maker(vendor: u32, name: &str) -> String {
    let maker = match vendor {
        0x8086 => "Intel",
        0x10de => "NVIDIA",
        0x1002 | 0x1022 => "AMD",
        _ => return name.to_owned(),
    };
    if name.to_lowercase().contains(&maker.to_lowercase()) { name.to_owned() } else { format!("{maker} {name}") }
}

fn pci_model(vendor: u32, device: u32) -> String {
    for path in ["/usr/share/hwdata/pci.ids", "/usr/share/misc/pci.ids", "/usr/share/pci.ids"] {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Some(name) = pci_ids_lookup(&text, vendor, device) {
                return with_maker(vendor, &name);
            }
        }
    }
    format!("{}:{device:04x}", vendor_name(vendor))
}

/// Every render node with the card behind it.
pub fn list() -> Vec<Gpu> {
    list_in(Path::new(DRM))
}

fn list_in(drm: &Path) -> Vec<Gpu> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(drm) else { return out };
    let mut names: Vec<String> = entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("renderD")).collect();
    names.sort();
    for name in names {
        let dev = drm.join(&name).join("device");
        let (Some(vendor), Some(device)) = (hex(&dev.join("vendor")), hex(&dev.join("device"))) else { continue };
        let driver = std::fs::read_link(dev.join("driver")).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
        let pci = std::fs::canonicalize(&dev).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
        // The card directory that shares the device: `<device>/drm/cardN`.
        let card = std::fs::read_dir(dev.join("drm"))
            .ok()
            .and_then(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).find(|n| n.starts_with("card")))
            .unwrap_or_default();
        out.push(Gpu { node: format!("/dev/dri/{name}"), vendor: vendor_name(vendor), model: pci_model(vendor, device), driver, pci, card, util: Util::default() });
    }
    out
}

// ---- how busy ----

/// What the counters said at the start of a sample.
pub struct Sampler {
    at: Instant,
    /// Per GPU (same order as the list): the RC6 residence in ms of an Intel card.
    rc6: Vec<Option<u64>>,
}

fn rc6_path(drm: &Path, g: &Gpu) -> Option<PathBuf> {
    let card = drm.join(&g.card);
    [card.join("gt/gt0/rc6_residency_ms"), card.join("power/rc6_residency_ms"), card.join("device/tile0/gt0/gtidle/idle_residency_ms")].into_iter().find(|p| p.exists())
}

pub fn begin(gpus: &[Gpu]) -> Sampler {
    begin_in(Path::new(DRM), gpus)
}

fn begin_in(drm: &Path, gpus: &[Gpu]) -> Sampler {
    let rc6 = gpus.iter().map(|g| if g.vendor == "intel" { rc6_path(drm, g).and_then(|p| read_num(&p)) } else { None }).collect();
    Sampler { at: Instant::now(), rc6 }
}

/// The share of `wall_ms` the card was awake, from how far its RC6 residence moved.
pub fn busy_from_rc6(before_ms: u64, after_ms: u64, wall_ms: f64) -> f64 {
    if wall_ms <= 0.0 {
        return 0.0;
    }
    (100.0 * (1.0 - after_ms.saturating_sub(before_ms) as f64 / wall_ms)).clamp(0.0, 100.0)
}

/// Fills in the utilisation of every card, measured since `begin`.
pub fn finish(sampler: Sampler, gpus: &mut [Gpu]) {
    finish_in(Path::new(DRM), sampler, gpus)
}

fn finish_in(drm: &Path, sampler: Sampler, gpus: &mut [Gpu]) {
    let wall_ms = sampler.at.elapsed().as_secs_f64() * 1000.0;
    let nvml = if gpus.iter().any(|g| g.vendor == "nvidia") { Nvml::load() } else { None };
    for (i, g) in gpus.iter_mut().enumerate() {
        let card = drm.join(&g.card);
        let dev = drm.join(&g.card).join("device");
        match g.vendor.as_str() {
            "intel" => {
                if let (Some(before), Some(p)) = (sampler.rc6.get(i).copied().flatten(), rc6_path(drm, g)) {
                    if let Some(after) = read_num(&p) {
                        g.util.busy = Some(busy_from_rc6(before, after, wall_ms));
                    }
                }
                // 0 is an idle card, not a clock worth printing.
                g.util.mhz = [card.join("gt_act_freq_mhz"), card.join("gt/gt0/rps_act_freq_mhz")].iter().find_map(|p| read_num(p)).filter(|m| *m > 0).map(|m| m as u32);
            }
            "amd" => {
                g.util.busy = read_num(&dev.join("gpu_busy_percent")).map(|b| b as f64);
                g.util.mem_used_mb = read_num(&dev.join("mem_info_vram_used")).map(|b| b / 1_048_576);
                g.util.mem_total_mb = read_num(&dev.join("mem_info_vram_total")).map(|b| b / 1_048_576);
                g.util.watts = std::fs::read_dir(dev.join("hwmon")).ok().and_then(|d| d.flatten().find_map(|e| read_num(&e.path().join("power1_average")))).map(|uw| uw as f64 / 1e6);
            }
            "nvidia" => {
                if let Some(n) = &nvml {
                    if let Some((name, util)) = n.read(&g.pci) {
                        if !name.is_empty() {
                            g.model = name;
                        }
                        g.util = util;
                    }
                }
            }
            _ => {}
        }
    }
}

// ---- NVIDIA: NVML, loaded when needed ----

#[repr(C)]
#[derive(Default)]
struct NvmlUtilization {
    gpu: c_uint,
    memory: c_uint,
}

#[repr(C)]
#[derive(Default)]
struct NvmlMemory {
    total: c_ulonglong,
    free: c_ulonglong,
    used: c_ulonglong,
}

struct Nvml {
    lib: libloading::Library,
}

type Handle = *mut c_void;

impl Nvml {
    fn load() -> Option<Self> {
        // SAFETY: loading NVIDIA's management library runs its initialisers, as any use of it does.
        let lib = unsafe { libloading::Library::new("libnvidia-ml.so.1") }.ok()?;
        let init: libloading::Symbol<unsafe extern "C" fn() -> c_int> = unsafe { lib.get(b"nvmlInit_v2\0") }.ok()?;
        // SAFETY: no arguments; 0 is NVML_SUCCESS.
        if unsafe { init() } != 0 {
            return None;
        }
        Some(Self { lib })
    }

    /// The model name and utilisation of the card at the PCI address `pci` (`0000:01:00.0`).
    fn read(&self, pci: &str) -> Option<(String, Util)> {
        // SAFETY: each symbol is looked up with the signature NVML documents; the buffers are the sizes it is told.
        unsafe {
            let by_bus: libloading::Symbol<unsafe extern "C" fn(*const c_char, *mut Handle) -> c_int> = self.lib.get(b"nvmlDeviceGetHandleByPciBusId_v2\0").ok()?;
            let bus = CString::new(pci).ok()?;
            let mut h: Handle = std::ptr::null_mut();
            if by_bus(bus.as_ptr(), &mut h) != 0 {
                return None;
            }
            let mut name = [0 as c_char; 96];
            let name_fn: libloading::Symbol<unsafe extern "C" fn(Handle, *mut c_char, c_uint) -> c_int> = self.lib.get(b"nvmlDeviceGetName\0").ok()?;
            let model = if name_fn(h, name.as_mut_ptr(), name.len() as c_uint) == 0 { std::ffi::CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned() } else { String::new() };
            let mut util = Util::default();
            let mut u = NvmlUtilization::default();
            if let Ok(f) = self.lib.get::<unsafe extern "C" fn(Handle, *mut NvmlUtilization) -> c_int>(b"nvmlDeviceGetUtilizationRates\0") {
                if f(h, &mut u) == 0 {
                    util.busy = Some(f64::from(u.gpu));
                }
            }
            let (mut enc, mut period): (c_uint, c_uint) = (0, 0);
            if let Ok(f) = self.lib.get::<unsafe extern "C" fn(Handle, *mut c_uint, *mut c_uint) -> c_int>(b"nvmlDeviceGetEncoderUtilization\0") {
                if f(h, &mut enc, &mut period) == 0 {
                    util.encoder = Some(f64::from(enc));
                }
            }
            let mut m = NvmlMemory::default();
            if let Ok(f) = self.lib.get::<unsafe extern "C" fn(Handle, *mut NvmlMemory) -> c_int>(b"nvmlDeviceGetMemoryInfo\0") {
                if f(h, &mut m) == 0 {
                    util.mem_used_mb = Some(m.used / 1_048_576);
                    util.mem_total_mb = Some(m.total / 1_048_576);
                }
            }
            let mut mw: c_uint = 0;
            if let Ok(f) = self.lib.get::<unsafe extern "C" fn(Handle, *mut c_uint) -> c_int>(b"nvmlDeviceGetPowerUsage\0") {
                if f(h, &mut mw) == 0 {
                    util.watts = Some(f64::from(mw) / 1000.0);
                }
            }
            let mut mhz: c_uint = 0;
            if let Ok(f) = self.lib.get::<unsafe extern "C" fn(Handle, c_uint, *mut c_uint) -> c_int>(b"nvmlDeviceGetClockInfo\0") {
                // 0: the graphics clock.
                if f(h, 0, &mut mhz) == 0 {
                    util.mhz = Some(mhz);
                }
            }
            Some((model, util))
        }
    }
}

impl Drop for Nvml {
    fn drop(&mut self) {
        // SAFETY: the matching shutdown of `nvmlInit_v2`.
        unsafe {
            if let Ok(f) = self.lib.get::<unsafe extern "C" fn() -> c_int>(b"nvmlShutdown\0") {
                f();
            }
        }
    }
}

/// One line for a card: "NVIDIA RTX 4000 SFF Ada Generation (nvidia, renderD129): 41 % busy, encoder 4 %, 508 MB of 20 GB, 12 W".
pub fn describe(g: &Gpu) -> String {
    let node = g.node.rsplit('/').next().unwrap_or(&g.node);
    let mut parts: Vec<String> = Vec::new();
    if let Some(b) = g.util.busy {
        parts.push(format!("{b:.0} % busy"));
    }
    if let Some(e) = g.util.encoder {
        parts.push(format!("encoder {e:.0} %"));
    }
    if let (Some(used), Some(total)) = (g.util.mem_used_mb, g.util.mem_total_mb) {
        let gb = |mb: u64| if mb >= 1024 { format!("{:.0} GB", mb as f64 / 1024.0) } else { format!("{mb} MB") };
        parts.push(format!("{} of {}", gb(used), gb(total)));
    }
    if let Some(m) = g.util.mhz {
        parts.push(format!("{m} MHz"));
    }
    if let Some(w) = g.util.watts {
        parts.push(format!("{w:.0} W"));
    }
    let load = if parts.is_empty() { String::new() } else { format!(": {}", parts.join(", ")) };
    format!("{} ({}, {node}){load}", g.model, g.driver)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDS: &str = "# a comment\n8086  Intel Corporation\n\t4680  AlderLake-S GT1\n\t\t1043 8882  A sub-device\n\t5912  HD Graphics 630\n10de  NVIDIA Corporation\n\t27b0  AD104GL [RTX 4000 SFF Ada Generation]\n";

    #[test]
    fn names_come_from_the_pci_id_database() {
        assert_eq!(pci_ids_lookup(IDS, 0x8086, 0x4680).as_deref(), Some("AlderLake-S GT1"));
        assert_eq!(pci_ids_lookup(IDS, 0x8086, 0x5912).as_deref(), Some("HD Graphics 630"));
        assert_eq!(pci_ids_lookup(IDS, 0x10de, 0x27b0).as_deref(), Some("AD104GL [RTX 4000 SFF Ada Generation]"));
        assert_eq!(pci_ids_lookup(IDS, 0x10de, 0x4680), None, "a device id of another vendor is not matched");
        assert_eq!(pci_ids_lookup(IDS, 0x8086, 0x1043), None, "a sub-device is not a device");
    }

    #[test]
    fn the_maker_is_put_in_front_of_a_name_that_lacks_it() {
        assert_eq!(with_maker(0x8086, "AlderLake-S GT1"), "Intel AlderLake-S GT1");
        assert_eq!(with_maker(0x10de, "NVIDIA RTX 4000"), "NVIDIA RTX 4000");
        assert_eq!(with_maker(0x1002, "Navi 31"), "AMD Navi 31");
        assert_eq!(with_maker(0x1234, "Something"), "Something");
    }

    #[test]
    fn busy_is_the_time_out_of_rc6() {
        assert_eq!(busy_from_rc6(1000, 2000, 1000.0), 0.0, "asleep the whole second");
        assert_eq!(busy_from_rc6(1000, 1000, 1000.0), 100.0, "never asleep");
        assert!((busy_from_rc6(0, 600, 1000.0) - 40.0).abs() < 1e-9);
        assert_eq!(busy_from_rc6(0, 5000, 1000.0), 0.0, "a residence that grew faster than the clock is not negative load");
        assert_eq!(busy_from_rc6(10, 5, 1000.0), 100.0, "a counter that went back is not a reason to report idle");
    }

    #[test]
    fn a_card_is_described_on_one_line() {
        let g = Gpu {
            node: "/dev/dri/renderD129".into(),
            vendor: "nvidia".into(),
            model: "RTX 4000".into(),
            driver: "nvidia".into(),
            util: Util { busy: Some(41.2), encoder: Some(4.0), mem_used_mb: Some(508), mem_total_mb: Some(20475), mhz: None, watts: Some(12.4) },
            ..Default::default()
        };
        assert_eq!(describe(&g), "RTX 4000 (nvidia, renderD129): 41 % busy, encoder 4 %, 508 MB of 20 GB, 12 W");
        let idle = Gpu { node: "/dev/dri/renderD128".into(), model: "UHD".into(), driver: "i915".into(), ..Default::default() };
        assert_eq!(describe(&idle), "UHD (i915, renderD128)");
    }

    #[test]
    fn a_machine_without_drm_has_no_gpus() {
        assert!(list_in(Path::new("/nonexistent-drm")).is_empty());
    }
}
