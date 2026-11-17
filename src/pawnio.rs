//! Cliente mínimo do driver PawnIO (https://github.com/namazso/PawnIO).
//!
//! O driver é assinado e executa módulos assinados (`IntelMSR.bin`, `AMDFamily17.bin`...)
//! em vez de expor acesso bruto ao hardware, ao contrário do WinRing0 (CVE-2020-14979).
//! Falamos com ele apenas por IOCTL, sem linkar nenhum código GPL.

use std::{ffi::c_void, fmt};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE},
        Storage::FileSystem::{CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING},
        System::IO::DeviceIoControl,
    },
};

const DEVICE_TYPE: u32 = 41_394 << 16;
const IOCTL_LOAD_BINARY: u32 = DEVICE_TYPE | (0x821 << 2);
const IOCTL_EXECUTE_FN: u32 = DEVICE_TYPE | (0x841 << 2);
const FN_NAME_LEN: usize = 32;

#[derive(Debug)]
pub enum Error {
    /// Driver não instalado (dispositivo `\Device\PawnIO` inexistente).
    NotInstalled,
    ModuleRead(std::io::Error),
    ModuleLoad(windows::core::Error),
    /// CPU sem suporte (fabricante/família desconhecidos).
    Unsupported(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotInstalled => write!(f, "driver PawnIO não instalado"),
            Error::ModuleRead(e) => write!(f, "não foi possível ler o módulo: {e}"),
            Error::ModuleLoad(e) => write!(f, "driver recusou o módulo: {e}"),
            Error::Unsupported(s) => write!(f, "CPU sem suporte: {s}"),
        }
    }
}

/// Um módulo PawnIO carregado (cada handle tem a sua própria instância do módulo).
pub struct Module(HANDLE);

impl Module {
    pub fn load(path: &std::path::Path) -> Result<Self, Error> {
        let bin = std::fs::read(path).map_err(Error::ModuleRead)?;
        let dev: Vec<u16> = r"\\?\GLOBALROOT\Device\PawnIO".encode_utf16().chain(Some(0)).collect();
        let h = unsafe {
            CreateFileW(
                PCWSTR(dev.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        }
        .map_err(|_| Error::NotInstalled)?;

        let mut ret = 0u32;
        let r = unsafe {
            DeviceIoControl(h, IOCTL_LOAD_BINARY, Some(bin.as_ptr() as *const c_void), bin.len() as u32, None, 0, Some(&mut ret), None)
        };
        if let Err(e) = r {
            unsafe { let _ = CloseHandle(h); }
            return Err(Error::ModuleLoad(e));
        }
        Ok(Self(h))
    }

    /// Executa `name(input...)` no módulo e devolve até `out_len` valores de 64 bits.
    pub fn call(&self, name: &str, input: &[u64], out_len: usize) -> Option<Vec<u64>> {
        let mut inbuf = vec![0u8; FN_NAME_LEN + input.len() * 8];
        let n = name.len().min(FN_NAME_LEN - 1);
        inbuf[..n].copy_from_slice(&name.as_bytes()[..n]);
        for (i, v) in input.iter().enumerate() {
            inbuf[FN_NAME_LEN + i * 8..][..8].copy_from_slice(&v.to_le_bytes());
        }
        let mut out = vec![0u8; out_len * 8];
        let mut ret = 0u32;
        unsafe {
            DeviceIoControl(
                self.0,
                IOCTL_EXECUTE_FN,
                Some(inbuf.as_ptr() as *const c_void),
                inbuf.len() as u32,
                Some(out.as_mut_ptr() as *mut c_void),
                out.len() as u32,
                Some(&mut ret),
                None,
            )
        }
        .ok()?;
        Some(
            out[..ret as usize]
                .chunks_exact(8)
                .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
                .collect(),
        )
    }
}

impl Drop for Module {
    fn drop(&mut self) {
        unsafe { let _ = CloseHandle(self.0); }
    }
}
