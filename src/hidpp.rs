//! Bateria de periféricos Logitech via HID++ 2.0 (mouse/teclado/headset, por receptor USB,
//! Bluetooth ou cabo). Fala direto com a coleção HID++ de cada dispositivo, sem depender do G HUB
//! (que continua podendo rodar em paralelo: cada handle recebe a sua cópia dos relatórios).

use std::{ffi::c_void, mem::size_of};
use windows::{
    core::{GUID, PCWSTR},
    Win32::{
        Devices::{
            DeviceAndDriverInstallation::*,
            HumanInterfaceDevice::*,
        },
        Foundation::*,
        Storage::FileSystem::*,
        System::{Threading::*, IO::*},
    },
};

const LOGITECH_VID: u16 = 0x046D;
const SWID: u8 = 0x0D; // identifica nossas requisições entre o tráfego de outros programas
const LONG_REPORT: u8 = 0x11;

#[derive(Clone, Debug, PartialEq)]
pub struct Battery {
    pub name: String,
    pub kind: Kind,
    pub percent: Option<u8>,
    pub charging: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Keyboard,
    Mouse,
    Headset,
    Other,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Keyboard => "KB",
            Kind::Mouse => "Mouse",
            Kind::Headset => "Headset",
            Kind::Other => "Dev",
        }
    }
    fn from_hidpp(t: u8) -> Self {
        match t {
            0 | 2 => Kind::Keyboard,
            3 | 4 | 5 => Kind::Mouse,
            8 | 9 => Kind::Headset,
            _ => Kind::Other,
        }
    }
}

struct Dev {
    h: HANDLE,
    ev: HANDLE,
    in_len: usize,
    out_len: usize,
}

impl Drop for Dev {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.ev);
            let _ = CloseHandle(self.h);
        }
    }
}

fn hid_paths() -> Vec<Vec<u16>> {
    unsafe {
        let guid: GUID = HidD_GetHidGuid();
        let mut len = 0u32;
        if CM_Get_Device_Interface_List_SizeW(&mut len, &guid, PCWSTR::null(), CM_GET_DEVICE_INTERFACE_LIST_PRESENT) != CR_SUCCESS || len < 2 {
            return vec![];
        }
        let mut buf = vec![0u16; len as usize];
        if CM_Get_Device_Interface_ListW(&guid, PCWSTR::null(), &mut buf, CM_GET_DEVICE_INTERFACE_LIST_PRESENT) != CR_SUCCESS {
            return vec![];
        }
        buf.split(|&c| c == 0).filter(|s| !s.is_empty()).map(|s| s.iter().copied().chain(Some(0)).collect()).collect()
    }
}

struct Info {
    is_receiver: bool,
}

/// Abre o dispositivo só para consulta (acesso 0) e diz se é uma coleção HID++ longa da Logitech.
fn inspect(path: &[u16]) -> Option<(Info, usize, usize)> {
    unsafe {
        let h = CreateFileW(PCWSTR(path.as_ptr()), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, None).ok()?;
        let result = (|| {
            let mut attr = HIDD_ATTRIBUTES { Size: size_of::<HIDD_ATTRIBUTES>() as u32, ..Default::default() };
            if !HidD_GetAttributes(h, &mut attr) || attr.VendorID != LOGITECH_VID {
                return None;
            }
            let mut pp = PHIDP_PREPARSED_DATA::default();
            if !HidD_GetPreparsedData(h, &mut pp) {
                return None;
            }
            let mut caps = HIDP_CAPS::default();
            let ok = HidP_GetCaps(pp, &mut caps) == HIDP_STATUS_SUCCESS;
            let _ = HidD_FreePreparsedData(pp);
            #[cfg(debug_assertions)]
            eprintln!(
                "[hid] pid={:04X} page={:04X} usage={:04X} in={} out={} feat={}",
                attr.ProductID, caps.UsagePage, caps.Usage, caps.InputReportByteLength, caps.OutputReportByteLength, caps.FeatureReportByteLength
            );
            if !ok || !(caps.UsagePage == 0xFF00 || caps.UsagePage == 0xFF43) || !matches!(caps.OutputReportByteLength, 7 | 20) {
                return None;
            }
            let mut name = [0u16; 126];
            let _ = HidD_GetProductString(h, name.as_mut_ptr() as *mut c_void, size_of::<[u16; 126]>() as u32);
            let n = String::from_utf16_lossy(&name).trim_end_matches('\0').to_lowercase();
            Some((Info { is_receiver: n.contains("receiver") }, caps.InputReportByteLength as usize, caps.OutputReportByteLength as usize))
        })();
        let _ = CloseHandle(h);
        result
    }
}

fn open(path: &[u16], in_len: usize, out_len: usize) -> Option<Dev> {
    unsafe {
        let h = CreateFileW(
            PCWSTR(path.as_ptr()),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            None,
        )
        .ok()?;
        let ev = CreateEventW(None, true, false, None).ok()?;
        Some(Dev { h, ev, in_len, out_len })
    }
}

impl Dev {
    fn write(&self, data: &[u8]) -> bool {
        unsafe {
            let mut buf = vec![0u8; self.out_len];
            buf[..data.len().min(self.out_len)].copy_from_slice(&data[..data.len().min(self.out_len)]);
            let _ = ResetEvent(self.ev);
            let mut ov = OVERLAPPED { hEvent: self.ev, ..Default::default() };
            let r = WriteFile(self.h, Some(&buf), None, Some(&mut ov));
            if r.is_err() && GetLastError() != ERROR_IO_PENDING {
                return false;
            }
            let mut n = 0u32;
            WaitForSingleObject(self.ev, 500) == WAIT_OBJECT_0 && GetOverlappedResult(self.h, &ov, &mut n, false).is_ok()
        }
    }

    fn read(&self, ms: u32) -> Option<Vec<u8>> {
        unsafe {
            let mut buf = vec![0u8; self.in_len];
            let _ = ResetEvent(self.ev);
            let mut ov = OVERLAPPED { hEvent: self.ev, ..Default::default() };
            let r = ReadFile(self.h, Some(&mut buf), None, Some(&mut ov));
            if r.is_err() && GetLastError() != ERROR_IO_PENDING {
                return None;
            }
            let mut n = 0u32;
            if WaitForSingleObject(self.ev, ms) != WAIT_OBJECT_0 {
                let _ = CancelIoEx(self.h, Some(&ov));
                let _ = GetOverlappedResult(self.h, &ov, &mut n, true);
                return None;
            }
            GetOverlappedResult(self.h, &ov, &mut n, false).ok()?;
            buf.truncate(n as usize);
            Some(buf)
        }
    }

    /// Chamada HID++ 2.0: devolve os 16 bytes de parâmetros da resposta.
    fn call(&self, idx: u8, feat: u8, func: u8, params: &[u8], timeout_ms: u32) -> Option<Vec<u8>> {
        let mut req = [0u8; 20];
        req[0] = LONG_REPORT;
        req[1] = idx;
        req[2] = feat;
        req[3] = (func << 4) | SWID;
        req[4..4 + params.len()].copy_from_slice(params);
        if !self.write(&req) {
            return None;
        }
        let t0 = std::time::Instant::now();
        while t0.elapsed().as_millis() < timeout_ms as u128 {
            let left = timeout_ms.saturating_sub(t0.elapsed().as_millis() as u32).max(1);
            let r = self.read(left)?;
            if r.len() < 5 || r[1] != idx {
                #[cfg(debug_assertions)]
                eprintln!("[hid]   ignorado (esperava idx={idx:02X}): {:02X?}", &r[..r.len().min(10)]);
                continue;
            }
            if r[2] == feat && r[3] == req[3] {
                return Some(r[4..].to_vec());
            }
            if (r[2] == 0xFF && r[3] == feat && r[4] == req[3]) || r[2] == 0x8F {
                #[cfg(debug_assertions)]
                eprintln!("[hid]   erro idx={idx:02X} feat={feat:02X} func={func}: {:02X?}", &r[..r.len().min(9)]);
                return None; // erro HID++ (feature inexistente, dispositivo offline...)
            }
        }
        None
    }

    fn feature(&self, idx: u8, id: u16, timeout_ms: u32) -> Option<u8> {
        let r = self.call(idx, 0x00, 0, &id.to_be_bytes(), timeout_ms)?;
        (r[0] != 0).then_some(r[0])
    }

    fn name_and_kind(&self, idx: u8) -> (String, Kind) {
        let Some(f) = self.feature(idx, 0x0005, 500) else { return ("Logitech".into(), Kind::Other) };
        let kind = self.call(idx, f, 2, &[], 500).map_or(Kind::Other, |r| Kind::from_hidpp(r[0]));
        let count = self.call(idx, f, 0, &[], 500).map_or(0, |r| r[0] as usize);
        let mut name = Vec::new();
        while name.len() < count {
            let Some(r) = self.call(idx, f, 1, &[name.len() as u8], 500) else { break };
            let take = (count - name.len()).min(16);
            name.extend_from_slice(&r[..take]);
        }
        let name = String::from_utf8_lossy(&name).trim_matches('\0').trim().to_string();
        (if name.is_empty() { "Logitech".into() } else { name }, kind)
    }

    fn battery(&self, idx: u8) -> Option<(u8, bool)> {
        if let Some(f) = self.feature(idx, 0x1004, 500) {
            let r = self.call(idx, f, 1, &[], 500)?; // GetStatus: % , nível, estado
            return Some((r[0].min(100), matches!(r[2], 1..=3)));
        }
        if let Some(f) = self.feature(idx, 0x1000, 500) {
            let r = self.call(idx, f, 0, &[], 500)?; // GetBatteryLevelStatus
            return Some((r[0].min(100), matches!(r[2], 1..=4)));
        }
        if let Some(f) = self.feature(idx, 0x1001, 500) {
            let r = self.call(idx, f, 0, &[], 500)?; // tensão em mV + flags
            let mv = u16::from_be_bytes([r[0], r[1]]);
            return Some((voltage_to_percent(mv), r[2] & 0x80 != 0));
        }
        if let Some(f) = self.feature(idx, 0x1F20, 500) {
            // ADC Measurement (ex.: G733): tensão em mV (aproximada) + flags de carga.
            for func in 0..4u8 {
                let r = self.call(idx, f, func, &[], 500);
                #[cfg(debug_assertions)]
                eprintln!("[hid]   ADC fn{func}: {:02X?}", r.as_ref().map(|r| &r[..8]));
                if let (1, Some(r)) = (func, r) {
                    let mv = u16::from_be_bytes([r[0], r[1]]);
                    return Some((voltage_to_percent(mv), r[2] & 0x01 != 0));
                }
            }
            return None;
        }
        None
    }
}

/// Curva de descarga típica de Li-Po de 1 célula (mV → %).
fn voltage_to_percent(mv: u16) -> u8 {
    const T: [(u16, u8); 13] = [
        (4186, 100), (4067, 90), (3989, 80), (3922, 70), (3859, 60), (3811, 50), (3778, 40),
        (3751, 30), (3717, 20), (3671, 10), (3646, 5), (3579, 2), (3500, 0),
    ];
    if mv >= T[0].0 {
        return 100;
    }
    for w in T.windows(2) {
        let ((hi_v, hi_p), (lo_v, lo_p)) = (w[0], w[1]);
        if mv >= lo_v {
            let f = (mv - lo_v) as f32 / (hi_v - lo_v) as f32;
            return (lo_p as f32 + f * (hi_p - lo_p) as f32).round() as u8;
        }
    }
    0
}

/// Diagnóstico: lê registradores HID++ 1.0 do receptor pela coleção de relatórios curtos.
#[cfg(debug_assertions)]
fn debug_receiver_short(dev: &Dev) {
    let reg = |sub: u8, r: u8, p: [u8; 3]| {
        if !dev.write(&[0x10, 0xFF, sub, r, p[0], p[1], p[2]]) {
            eprintln!("[rx] write falhou reg {r:02X}");
            return;
        }
        let t0 = std::time::Instant::now();
        while t0.elapsed().as_millis() < 600 {
            match dev.read(300) {
                Some(b) => {
                    eprintln!("[rx] reg {r:02X}{p:02X?} -> {:02X?}", b);
                    if b.len() >= 4 && (b[2] == sub || b[2] == 0x8F) {
                        break;
                    }
                }
                None => break,
            }
        }
    };
    reg(0x81, 0x02, [0, 0, 0]); // estado de conexão
    for n in 0..6u8 {
        reg(0x83, 0xB5, [0x20 + n, 0, 0]); // info de pareamento do slot n+1
    }
}

/// Varre todos os dispositivos Logitech HID++ e devolve a bateria de cada um.
pub fn scan() -> Vec<Battery> {
    let mut out: Vec<Battery> = Vec::new();
    for path in hid_paths() {
        let Some((info, in_len, out_len)) = inspect(&path) else { continue };
        let Some(dev) = open(&path, in_len, out_len) else { continue };
        if out_len == 7 {
            #[cfg(debug_assertions)]
            debug_receiver_short(&dev);
            continue; // coleção de relatórios curtos: só diagnóstico
        }
        let indices: &[u8] = if info.is_receiver { &[1, 2, 3, 4, 5, 6] } else { &[0xFF, 0x00] };
        for &idx in indices {
            // Ping (IRoot.getProtocolVersion): só segue se houver dispositivo respondendo.
            // Dispositivo em repouso demora a acordar: receptores ganham mais tempo.
            let ping_ms = if info.is_receiver { 1200 } else { 250 };
            if dev.call(idx, 0x00, 1, &[0, 0, 0x5A], ping_ms).is_none() {
                #[cfg(debug_assertions)]
                eprintln!("[hid] idx {idx:02X}: sem resposta");
                continue;
            }
            #[cfg(debug_assertions)]
            eprintln!("[hid] idx {idx:02X}: respondeu");
            #[cfg(debug_assertions)]
            if let Some(fs) = dev.feature(idx, 0x0001, 500) {
                let n = dev.call(idx, fs, 0, &[], 500).map_or(0, |r| r[0]);
                let mut ids = Vec::new();
                for i in 1..=n {
                    if let Some(r) = dev.call(idx, fs, 1, &[i], 500) {
                        ids.push(format!("{:02X}:{:04X}", i, u16::from_be_bytes([r[0], r[1]])));
                    }
                }
                eprintln!("[hid]   features idx={idx:02X}: {}", ids.join(" "));
            }
            let (name, kind) = dev.name_and_kind(idx);
            #[cfg(debug_assertions)]
            eprintln!("[hid]   nome={name:?} tipo={kind:?}");
            let Some((percent, charging)) = dev.battery(idx) else { continue };
            let b = Battery { name, kind, percent: Some(percent), charging };
            if !out.iter().any(|o| o.name == b.name && o.kind == b.kind) {
                out.push(b);
            }
        }
    }
    out
}
