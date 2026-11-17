//! FPS do processo em primeiro plano, contando eventos `Present` via ETW (a mesma fonte do PresentMon).
//! Provedores: Microsoft-Windows-DXGI (D3D10/11/12) e Microsoft-Windows-D3D9. Exige administrador.
//!
//! Limite conhecido: jogos Vulkan/OpenGL que não passam pelo DXGI não geram esses eventos.

use std::{
    collections::{HashMap, VecDeque},
    mem::size_of,
    sync::{Mutex, OnceLock},
    time::Instant,
};
use windows::{
    core::{GUID, PWSTR},
    Win32::{
        Foundation::*,
        System::Diagnostics::Etw::*,
        UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
    },
};

const SESSION_NAME: &str = "SensoryxFpsSession";
const DXGI: GUID = GUID::from_u128(0xCA11C036_0102_4A2D_A6AD_F03CFED5D3C9);
const D3D9: GUID = GUID::from_u128(0x783ACA0A_790E_4D7F_8451_AA850511C6B9);
const DXGI_PRESENT_START: u16 = 42;
const D3D9_PRESENT: u16 = 1;
const WINDOW_MS: u64 = 1000;

static START: OnceLock<Instant> = OnceLock::new();
static EVENTS: Mutex<Option<HashMap<u32, VecDeque<u64>>>> = Mutex::new(None);
static SESSION: Mutex<Option<u64>> = Mutex::new(None);

fn now_ms() -> u64 {
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

unsafe extern "system" fn on_event(rec: *mut EVENT_RECORD) {
    let h = &(*rec).EventHeader;
    let id = h.EventDescriptor.Id;
    let hit = (h.ProviderId == DXGI && id == DXGI_PRESENT_START) || (h.ProviderId == D3D9 && id == D3D9_PRESENT);
    if !hit {
        return;
    }
    let t = now_ms();
    if let Ok(mut g) = EVENTS.lock() {
        let q = g.get_or_insert_with(HashMap::new).entry(h.ProcessId).or_default();
        q.push_back(t);
        while q.front().is_some_and(|&x| t.saturating_sub(x) > 2 * WINDOW_MS) {
            q.pop_front();
        }
    }
}

fn session_props() -> Vec<u8> {
    let name_bytes = (SESSION_NAME.len() + 1) * 2;
    let total = size_of::<EVENT_TRACE_PROPERTIES>() + name_bytes;
    let mut buf = vec![0u8; total];
    let p = buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
    unsafe {
        (*p).Wnode.BufferSize = total as u32;
        (*p).Wnode.Flags = WNODE_FLAG_TRACED_GUID;
        (*p).Wnode.ClientContext = 1; // timestamps em QPC
        (*p).LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
        (*p).LoggerNameOffset = size_of::<EVENT_TRACE_PROPERTIES>() as u32;
        (*p).FlushTimer = 1;
    }
    buf
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Inicia a sessão ETW e o consumidor numa thread. Devolve `false` se não for possível
/// (normalmente por falta de privilégio de administrador).
pub fn start() -> bool {
    let _ = now_ms();
    unsafe {
        let name = wide(SESSION_NAME);
        let mut handle = CONTROLTRACE_HANDLE::default();
        let mut props = session_props();
        let mut r = StartTraceW(&mut handle, windows::core::PCWSTR(name.as_ptr()), props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES);
        if r == ERROR_ALREADY_EXISTS {
            // Sobrou de uma execução anterior que não encerrou direito: derruba e recria.
            let mut old = session_props();
            let _ = ControlTraceW(CONTROLTRACE_HANDLE::default(), windows::core::PCWSTR(name.as_ptr()), old.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES, EVENT_TRACE_CONTROL_STOP);
            props = session_props();
            r = StartTraceW(&mut handle, windows::core::PCWSTR(name.as_ptr()), props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES);
        }
        if r != ERROR_SUCCESS {
            return false;
        }
        *SESSION.lock().unwrap() = Some(handle.Value);
        for g in [&DXGI, &D3D9] {
            let _ = EnableTraceEx2(handle, g, EVENT_CONTROL_CODE_ENABLE_PROVIDER.0, TRACE_LEVEL_INFORMATION as u8, u64::MAX, 0, 0, None);
        }

        std::thread::spawn(move || {
            let mut name = wide(SESSION_NAME);
            let mut log = EVENT_TRACE_LOGFILEW { LoggerName: PWSTR(name.as_mut_ptr()), ..Default::default() };
            log.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
            log.Anonymous2.EventRecordCallback = Some(on_event);
            let h = OpenTraceW(&mut log);
            if h.Value == u64::MAX {
                return;
            }
            let _ = ProcessTrace(&[h], None, None); // bloqueia até a sessão ser encerrada
            let _ = CloseTrace(h);
        });
    }
    true
}

/// Encerra a sessão ETW (para não deixar sessão órfã no sistema).
pub fn stop() {
    if let Some(v) = SESSION.lock().unwrap().take() {
        unsafe {
            let name = wide(SESSION_NAME);
            let mut props = session_props();
            let _ = ControlTraceW(CONTROLTRACE_HANDLE { Value: v }, windows::core::PCWSTR(name.as_ptr()), props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES, EVENT_TRACE_CONTROL_STOP);
        }
    }
}

/// Quadros por segundo do processo em primeiro plano (`None` se ele não apresentou quadros).
pub fn foreground_fps() -> Option<u32> {
    let pid = unsafe {
        let hwnd = GetForegroundWindow();
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        pid
    };
    fps_of(pid)
}

fn fps_of(pid: u32) -> Option<u32> {
    let t = now_ms();
    let mut g = EVENTS.lock().ok()?;
    let map = g.as_mut()?;
    map.retain(|_, q| q.back().is_some_and(|&x| t.saturating_sub(x) <= 2 * WINDOW_MS));
    let n = map.get(&pid)?.iter().filter(|&&x| t.saturating_sub(x) <= WINDOW_MS).count() as u32;
    (n > 0).then_some(n)
}

/// Diagnóstico: FPS de todos os processos que estão apresentando quadros.
pub fn all_fps() -> Vec<(u32, u32)> {
    let t = now_ms();
    let Ok(g) = EVENTS.lock() else { return vec![] };
    let Some(map) = g.as_ref() else { return vec![] };
    let mut v: Vec<_> = map
        .iter()
        .map(|(&pid, q)| (pid, q.iter().filter(|&&x| t.saturating_sub(x) <= WINDOW_MS).count() as u32))
        .filter(|&(_, n)| n > 0)
        .collect();
    v.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
    v
}
