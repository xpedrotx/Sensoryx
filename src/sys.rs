//! Uso de CPU e memória RAM do sistema.

use windows::Win32::{
    Foundation::FILETIME,
    System::{SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX}, Threading::GetSystemTimes},
};

fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

/// Uso total da CPU (0–100 %), calculado entre duas chamadas consecutivas.
#[derive(Default)]
pub struct CpuLoad {
    prev: Option<(u64, u64)>, // (total, ocioso)
}

impl CpuLoad {
    pub fn usage(&mut self) -> Option<u32> {
        let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).ok()? };
        // `kernel` já inclui o tempo ocioso.
        let now = (ft(kernel) + ft(user), ft(idle));
        let out = self.prev.and_then(|(pt, pi)| {
            let total = now.0.checked_sub(pt)?;
            let idle = now.1.checked_sub(pi)?;
            (total > 0).then(|| (100 * total.saturating_sub(idle) / total).min(100) as u32)
        });
        self.prev = Some(now);
        out
    }
}

/// Memória RAM em uso, em MB.
pub fn ram_used_mb() -> Option<u32> {
    let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    unsafe { GlobalMemoryStatusEx(&mut m).ok()? };
    Some(((m.ullTotalPhys - m.ullAvailPhys) / (1024 * 1024)) as u32)
}
