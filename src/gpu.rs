//! Sensores da GPU NVIDIA via NVML (nvml.dll vem com o driver; carregada dinamicamente,
//! então o app continua funcionando em PCs sem NVIDIA).

use libloading::Library;
use std::ffi::c_void;

type NvmlReturn = i32;
type Handle = *mut c_void;

#[repr(C)]
#[derive(Default)]
struct Utilization {
    gpu: u32,
    memory: u32,
}

#[repr(C)]
#[derive(Default)]
struct MemoryInfo {
    total: u64,
    free: u64,
    used: u64,
}

type FnIdxHandle = unsafe extern "C" fn(u32, *mut Handle) -> NvmlReturn;
type FnU32 = unsafe extern "C" fn(Handle, *mut u32) -> NvmlReturn;
type FnU32Kind = unsafe extern "C" fn(Handle, u32, *mut u32) -> NvmlReturn;

/// Leituras instantâneas da GPU (`None` = a placa/driver não informa).
#[derive(Default, Clone)]
pub struct GpuStats {
    pub name: String,
    pub temp: Option<u32>,
    pub usage: Option<u32>,
    pub power_w: Option<u32>,
    pub vram_used_mb: Option<u32>,
    pub core_mhz: Option<u32>,
    pub mem_mhz: Option<u32>,
}

pub struct NvmlGpu {
    _lib: Library,
    device: Handle,
    name: String,
    get_temp: FnU32Kind,
    get_clock: Option<FnU32Kind>,
    get_power: Option<FnU32>,
    get_util: Option<unsafe extern "C" fn(Handle, *mut Utilization) -> NvmlReturn>,
    get_mem: Option<unsafe extern "C" fn(Handle, *mut MemoryInfo) -> NvmlReturn>,
    shutdown: unsafe extern "C" fn() -> NvmlReturn,
}

unsafe fn sym<T: Copy>(lib: &Library, name: &[u8]) -> Option<T> {
    lib.get::<T>(name).ok().map(|s| *s)
}

impl NvmlGpu {
    pub fn open() -> Option<Self> {
        unsafe {
            let lib = ["nvml.dll", r"C:\Program Files\NVIDIA Corporation\NVSMI\nvml.dll"]
                .iter()
                .find_map(|p| Library::new(p).ok())?;

            let init: unsafe extern "C" fn() -> NvmlReturn = sym(&lib, b"nvmlInit_v2\0")?;
            let get_handle: FnIdxHandle = sym(&lib, b"nvmlDeviceGetHandleByIndex_v2\0")?;
            let get_temp: FnU32Kind = sym(&lib, b"nvmlDeviceGetTemperature\0")?;
            let shutdown: unsafe extern "C" fn() -> NvmlReturn = sym(&lib, b"nvmlShutdown\0")?;
            let get_name: Option<unsafe extern "C" fn(Handle, *mut u8, u32) -> NvmlReturn> = sym(&lib, b"nvmlDeviceGetName\0");
            let get_clock: Option<FnU32Kind> = sym(&lib, b"nvmlDeviceGetClockInfo\0");
            let get_power: Option<FnU32> = sym(&lib, b"nvmlDeviceGetPowerUsage\0");
            let get_util = sym(&lib, b"nvmlDeviceGetUtilizationRates\0");
            let get_mem = sym(&lib, b"nvmlDeviceGetMemoryInfo\0");

            if init() != 0 {
                return None;
            }
            let mut device: Handle = std::ptr::null_mut();
            if get_handle(0, &mut device) != 0 {
                shutdown();
                return None;
            }
            let mut name = String::new();
            if let Some(f) = get_name {
                let mut buf = [0u8; 96];
                if f(device, buf.as_mut_ptr(), buf.len() as u32) == 0 {
                    let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                    name = String::from_utf8_lossy(&buf[..n]).into_owned();
                }
            }
            Some(Self { _lib: lib, device, name, get_temp, get_clock, get_power, get_util, get_mem, shutdown })
        }
    }

    /// Temperatura do núcleo da GPU em °C.
    pub fn temperature(&self) -> Option<u32> {
        let mut t = 0u32;
        // sensor 0 = NVML_TEMPERATURE_GPU
        (unsafe { (self.get_temp)(self.device, 0, &mut t) } == 0).then_some(t)
    }

    fn clock(&self, kind: u32) -> Option<u32> {
        let f = self.get_clock?;
        let mut v = 0u32;
        (unsafe { f(self.device, kind, &mut v) } == 0).then_some(v)
    }

    pub fn stats(&self) -> GpuStats {
        let usage = self.get_util.and_then(|f| {
            let mut u = Utilization::default();
            (unsafe { f(self.device, &mut u) } == 0).then_some(u.gpu)
        });
        let power_w = self.get_power.and_then(|f| {
            let mut mw = 0u32;
            (unsafe { f(self.device, &mut mw) } == 0).then_some((mw + 500) / 1000)
        });
        let vram_used_mb = self.get_mem.and_then(|f| {
            let mut m = MemoryInfo::default();
            (unsafe { f(self.device, &mut m) } == 0).then_some((m.used / (1024 * 1024)) as u32)
        });
        GpuStats {
            name: self.name.clone(),
            temp: self.temperature(),
            usage,
            power_w,
            vram_used_mb,
            core_mhz: self.clock(0), // NVML_CLOCK_GRAPHICS
            mem_mhz: self.clock(2),  // NVML_CLOCK_MEM
        }
    }
}

impl Drop for NvmlGpu {
    fn drop(&mut self) {
        unsafe { (self.shutdown)() };
    }
}
