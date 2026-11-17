//! Temperatura da CPU via MSR/SMN, usando módulos assinados do PawnIO.
//!
//! - Intel: `IntelMSR.bin` — `MSR_TEMPERATURE_TARGET` (0x1A2) bits 23:16 = TjMax e
//!   `IA32_PACKAGE_THERM_STATUS` (0x1B1) bits 22:16 = distância até o TjMax (°C).
//! - AMD Ryzen (família 17h–1Ah): `AMDFamily17.bin` — registrador SMN `THM_TCON_CUR_TMP` (0x59800).

use crate::pawnio::{Error, Module};
use std::{arch::x86_64::__cpuid, path::Path};
use windows::{
    core::w,
    Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0},
        System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
    },
};

pub enum Cpu {
    Intel(IntelCpu),
    Amd(AmdCpu),
}

impl Cpu {
    /// Detecta o fabricante e carrega o módulo certo de `modules_dir`.
    pub fn open(modules_dir: &Path) -> Result<Self, Error> {
        match vendor().as_str() {
            "GenuineIntel" => IntelCpu::open(&modules_dir.join("IntelMSR.bin")).map(Cpu::Intel),
            "AuthenticAMD" => AmdCpu::open(&modules_dir.join("AMDFamily17.bin")).map(Cpu::Amd),
            v => Err(Error::Unsupported(v.to_string())),
        }
    }

    /// Temperatura do pacote da CPU em °C.
    pub fn temperature(&self) -> Option<u32> {
        match self {
            Cpu::Intel(c) => c.package_temperature(),
            Cpu::Amd(c) => c.temperature(),
        }
    }
}

/// Diagnóstico (`--cpu`): fabricante, modelo e resultado de cada backend.
#[cfg(debug_assertions)]
pub fn diagnose(modules_dir: &Path) {
    let (f, m) = family_model();
    println!("fabricante={} marca={:?} família={:#x} modelo={:#x}", vendor(), brand(), f, m);
    match Cpu::open(modules_dir) {
        Ok(c) => println!("backend automático: {:?} °C", c.temperature()),
        Err(e) => println!("backend automático: {e}"),
    }
    match AmdCpu::open(&modules_dir.join("AMDFamily17.bin")) {
        Ok(c) => println!("forçando AMD: {:?} °C", c.temperature()),
        Err(e) => println!("forçando AMD: {e}"),
    }
}

fn vendor() -> String {
    let r = __cpuid(0);
    let b: Vec<u8> = [r.ebx, r.edx, r.ecx].iter().flat_map(|v| v.to_le_bytes()).collect();
    String::from_utf8_lossy(&b).into_owned()
}

/// Nome comercial da CPU (ex.: "12th Gen Intel(R) Core(TM) i5-12400F").
pub fn brand_name() -> String {
    brand()
}

fn brand() -> String {
    let mut b = Vec::new();
    for leaf in 0x8000_0002u32..=0x8000_0004 {
        let r = __cpuid(leaf);
        for v in [r.eax, r.ebx, r.ecx, r.edx] {
            b.extend_from_slice(&v.to_le_bytes());
        }
    }
    String::from_utf8_lossy(&b).trim_matches('\0').trim().to_string()
}

/// (família, modelo) já com os campos estendidos aplicados.
fn family_model() -> (u32, u32) {
    let eax = __cpuid(1).eax;
    let base_family = (eax >> 8) & 0xF;
    let family = if base_family == 0xF { base_family + ((eax >> 20) & 0xFF) } else { base_family };
    let model = if matches!(base_family, 0x6 | 0xF) { ((eax >> 4) & 0xF) | (((eax >> 16) & 0xF) << 4) } else { (eax >> 4) & 0xF };
    (family, model)
}

// ---------------------------------------------------------------- Intel

pub struct IntelCpu {
    module: Module,
    tj_max: u32,
}

const MSR_TEMPERATURE_TARGET: u64 = 0x1A2;
const MSR_PACKAGE_THERM_STATUS: u64 = 0x1B1;

impl IntelCpu {
    pub fn open(module_path: &Path) -> Result<Self, Error> {
        let module = Module::load(module_path)?;
        let tj_max = module
            .call("ioctl_read_msr", &[MSR_TEMPERATURE_TARGET], 1)
            .and_then(|v| v.first().copied())
            .map(|v| ((v >> 16) & 0xFF) as u32)
            .filter(|&t| (60..=130).contains(&t))
            .unwrap_or(100);
        Ok(Self { module, tj_max })
    }

    pub fn package_temperature(&self) -> Option<u32> {
        let v = *self.module.call("ioctl_read_msr", &[MSR_PACKAGE_THERM_STATUS], 1)?.first()?;
        let valid = (v >> 31) & 1 == 1; // bit 31: leitura digital válida
        let below_tj = ((v >> 16) & 0x7F) as u32;
        valid.then(|| self.tj_max.saturating_sub(below_tj))
    }
}

// ------------------------------------------------------------------ AMD

pub struct AmdCpu {
    module: Module,
    /// Mutex global que as ferramentas de monitoramento usam para não colidir no acesso PCI/SMN.
    pci_mutex: HANDLE,
    tctl_offset: f32,
}

const SMN_THM_TCON_CUR_TMP: u64 = 0x0005_9800;

impl AmdCpu {
    pub fn open(module_path: &Path) -> Result<Self, Error> {
        let module = Module::load(module_path)?;
        let pci_mutex = unsafe { CreateMutexW(None, false, w!("Global\\Access_PCI")) }
            .map_err(|e| Error::Unsupported(format!("mutex Access_PCI: {e}")))?;
        let (family, model) = family_model();
        Ok(Self { module, pci_mutex, tctl_offset: tctl_offset(family, model, &brand()) })
    }

    pub fn temperature(&self) -> Option<u32> {
        let raw = self.read_smn(SMN_THM_TCON_CUR_TMP)?;
        let t = decode_tctl(raw) - self.tctl_offset;
        (0.0..150.0).contains(&t).then(|| t.round() as u32)
    }

    fn read_smn(&self, addr: u64) -> Option<u32> {
        unsafe {
            let w = WaitForSingleObject(self.pci_mutex, 5000);
            if w != WAIT_OBJECT_0 && w != WAIT_ABANDONED {
                return None;
            }
            let r = self.module.call("ioctl_read_smn", &[addr], 1).and_then(|v| v.first().map(|&x| x as u32));
            let _ = ReleaseMutex(self.pci_mutex);
            r
        }
    }
}

impl Drop for AmdCpu {
    fn drop(&mut self) {
        unsafe { let _ = CloseHandle(self.pci_mutex); }
    }
}

/// Decodifica `THM_TCON_CUR_TMP`: bits 31:21 = temperatura em passos de 0,125 °C;
/// bit 19 = faixa estendida (-49 °C).
fn decode_tctl(raw: u32) -> f32 {
    let mut t = ((raw >> 21) & 0x7FF) as f32 / 8.0;
    if raw & 0x80000 != 0 {
        t -= 49.0;
    }
    t
}

/// Os primeiros Ryzen série X reportam Tctl com um deslocamento artificial acima da temperatura real.
fn tctl_offset(family: u32, model: u32, brand: &str) -> f32 {
    if family != 0x17 {
        return 0.0;
    }
    let b = brand.to_lowercase();
    if b.contains("threadripper") {
        return if b.contains("1900x") || b.contains("1920x") || b.contains("1950x") || b.contains("2920x") || b.contains("2950x") || b.contains("2970wx") || b.contains("2990wx") { 27.0 } else { 0.0 };
    }
    let is_x = b.contains(" 1600x") || b.contains(" 1700x") || b.contains(" 1800x") || b.contains(" 2600x") || b.contains(" 2700x");
    match (model, is_x) {
        (0x01, true) => 20.0, // Zen (1ª geração)
        (0x08, true) => 10.0, // Zen+ (2ª geração)
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_normal_range() {
        assert_eq!(decode_tctl(360 << 21), 45.0); // 45 °C
        assert_eq!(decode_tctl(((45 * 8) + 1) << 21), 45.125);
    }

    #[test]
    fn decode_extended_range() {
        // Faixa estendida: valor bruto = (T + 49) * 8, com o bit 19 ligado.
        assert_eq!(decode_tctl((((45 + 49) * 8) << 21) | 0x80000), 45.0);
    }

    #[test]
    fn offsets() {
        assert_eq!(tctl_offset(0x19, 0x21, "AMD Ryzen 7 5800X 8-Core Processor"), 0.0);
        assert_eq!(tctl_offset(0x17, 0x01, "AMD Ryzen 7 1700X Eight-Core Processor"), 20.0);
        assert_eq!(tctl_offset(0x17, 0x08, "AMD Ryzen 7 2700X Eight-Core Processor"), 10.0);
        assert_eq!(tctl_offset(0x17, 0x71, "AMD Ryzen 7 3700X 8-Core Processor"), 0.0);
        assert_eq!(tctl_offset(0x17, 0x01, "AMD Ryzen Threadripper 1950X 16-Core Processor"), 27.0);
    }
}
