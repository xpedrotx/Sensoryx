//! Janela sobreposta à barra de tarefas (alfa por pixel) + ícone na bandeja.
//! Tudo em GDI/Win32 puro: sem framework de UI, sem threads extras.

use std::{cell::RefCell, ffi::c_void, mem::size_of, ptr::null_mut};
use crate::panel::BarItem;
use crate::settings::{Corner, Settings};

type BarItemLine = Vec<BarItem>;
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{LibraryLoader::GetModuleHandleW, Registry::*},
        UI::{HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
    },
};

/// Leituras mais recentes dos sensores (`None` = indisponível).
#[derive(Default, Clone)]
pub struct Readings {
    pub cpu: Option<u32>,
    pub gpu: Option<u32>,
    pub fps: Option<u32>,
    pub batteries: Vec<crate::hidpp::Battery>,
    // Detalhes usados só pelo painel de benchmark.
    pub gpu_name: String,
    pub cpu_name: String,
    pub gpu_usage: Option<u32>,
    pub gpu_power_w: Option<u32>,
    pub gpu_vram_mb: Option<u32>,
    pub gpu_core_mhz: Option<u32>,
    pub gpu_mem_mhz: Option<u32>,
    pub cpu_usage: Option<u32>,
    pub ram_mb: Option<u32>,
}

const WM_TRAY: u32 = WM_APP + 1;
const TIMER_ID: usize = 1;
const CMD_EXIT: usize = 1;
const CMD_SETTINGS: usize = 2;
const HOTKEY_ID: i32 = 1;

struct App {
    sample: Box<dyn FnMut() -> Readings>,
    settings: Settings,
    taskbar_created: u32,
    tray_text: Option<String>,
    tray_added: bool,
    panel: HWND,
    panel_visible: bool,
    last: Readings,
    /// Últimos ~60 s de FPS, para o gráfico do painel.
    fps_hist: Vec<u32>,
    /// Data do `settings.ini` na última leitura (para recarregar quando a tela de configurações grava).
    settings_mtime: Option<std::time::SystemTime>,
}

thread_local! { static APP: RefCell<Option<App>> = const { RefCell::new(None) }; }

pub fn run(settings: Settings, sample: impl FnMut() -> Readings + 'static) {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let hmod = GetModuleHandleW(None).unwrap();
        let hinst = HINSTANCE(hmod.0);

        let class = w!("SensoryxOverlay");
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassExW(&wc);

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            w!("Sensoryx"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(hinst),
            None,
        )
        .unwrap();

        // Janela do painel de benchmark: transparente ao mouse e sem roubar o foco do jogo.
        let panel = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
            class,
            w!("Sensoryx Benchmark"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(hinst),
            None,
        )
        .unwrap();

        // Atalho global: não injeta nada no jogo, só recebe a tecla do sistema.
        let hotkey_ok = RegisterHotKey(
            Some(hwnd),
            HOTKEY_ID,
            HOT_KEY_MODIFIERS(settings.hotkey_mods | MOD_NOREPEAT.0),
            settings.hotkey_vk,
        )
        .is_ok();
        #[cfg(debug_assertions)]
        eprintln!("atalho {} registrado: {hotkey_ok}", settings.hotkey);
        let _ = hotkey_ok;

        APP.with(|a| {
            *a.borrow_mut() = Some(App {
                sample: Box::new(sample),
                settings,
                taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
                tray_text: None,
                tray_added: false,
                panel,
                panel_visible: false,
                last: Readings::default(),
                fps_hist: Vec::new(),
                settings_mtime: crate::settings::mtime(),
            })
        });

        tick(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(Some(hwnd), TIMER_ID, 1000, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            tick(hwnd);
            LRESULT(0)
        }
        WM_HOTKEY if wp.0 as i32 == HOTKEY_ID => {
            toggle_panel(hwnd);
            LRESULT(0)
        }
        WM_TRAY if lp.0 as u32 == WM_RBUTTONUP || lp.0 as u32 == WM_CONTEXTMENU => {
            show_menu(hwnd);
            LRESULT(0)
        }
        WM_TRAY if lp.0 as u32 == WM_LBUTTONUP => {
            open_settings();
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            show_menu(hwnd);
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_DESTROY => {
            let _ = UnregisterHotKey(Some(hwnd), HOTKEY_ID);
            let nid = tray_data(hwnd, None, "");
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => {
            // Explorer reiniciou: o ícone da bandeja some e precisa ser recriado.
            let recreate = APP.with(|a| {
                a.try_borrow_mut().ok().and_then(|mut g| {
                    g.as_mut().filter(|app| app.taskbar_created == msg).map(|app| {
                        app.tray_added = false;
                        app.tray_text = None;
                    })
                })
            });
            if recreate.is_some() {
                tick(hwnd);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }
}

unsafe fn show_menu(hwnd: HWND) {
    let menu = CreatePopupMenu().unwrap();
    let _ = AppendMenuW(menu, MF_STRING, CMD_SETTINGS, w!("Configurações"));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, CMD_EXIT, w!("Sair do Sensoryx"));
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let _ = SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, None, hwnd, None);
    let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
    let _ = DestroyMenu(menu);
    match cmd.0 as usize {
        CMD_EXIT => {
            let _ = DestroyWindow(hwnd);
        }
        CMD_SETTINGS => open_settings(),
        _ => {}
    }
}

/// Abre a tela de configurações em outro processo (herda a elevação, então não pede UAC).
fn open_settings() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--settings").spawn();
    }
}

fn temp(v: Option<u32>, s: &Settings) -> String {
    v.map_or("--".into(), |t| format!("{}{}", s.temp(t), s.unit()))
}

unsafe fn tick(hwnd: HWND) {
    let Some((r, tray_added, last_text, cfg)) = APP.with(|a| {
        let mut g = a.try_borrow_mut().ok()?;
        let app = g.as_mut()?;

        // A tela de configurações gravou o arquivo? Recarrega e reaplica o atalho.
        let m = crate::settings::mtime();
        if m != app.settings_mtime {
            app.settings_mtime = m;
            let new = crate::settings::load();
            if (new.hotkey_mods, new.hotkey_vk) != (app.settings.hotkey_mods, app.settings.hotkey_vk) {
                let _ = UnregisterHotKey(Some(hwnd), HOTKEY_ID);
                let _ = RegisterHotKey(Some(hwnd), HOTKEY_ID, HOT_KEY_MODIFIERS(new.hotkey_mods | MOD_NOREPEAT.0), new.hotkey_vk);
            }
            app.settings = new;
            app.tray_text = None; // força redesenhar o ícone (unidade de temperatura)
        }

        let r = (app.sample)();
        app.last = r.clone();
        match r.fps {
            Some(f) => {
                app.fps_hist.push(f);
                if app.fps_hist.len() > 60 {
                    app.fps_hist.remove(0);
                }
            }
            None => app.fps_hist.clear(), // sem jogo em primeiro plano: reinicia o gráfico
        }
        Some((r, app.tray_added, app.tray_text.clone(), app.settings.clone()))
    }) else {
        return;
    };

    // Linhas da barra, conforme o que o usuário escolheu mostrar.
    let mut line1 = Vec::new();
    if cfg.bar_cpu {
        line1.push(BarItem::Text(format!("CPU {}", temp(r.cpu, &cfg))));
    }
    if cfg.bar_gpu {
        line1.push(BarItem::Text(format!("GPU {}", temp(r.gpu, &cfg))));
    }
    let mut line2 = Vec::new();
    if cfg.bar_fps {
        line2.push(BarItem::Text(format!("FPS {}", r.fps.map_or("--".into(), |f| f.to_string()))));
    }
    if cfg.bar_devices {
        // Periféricos aparecem como ícone + porcentagem (sem o nome).
        for b in &r.batteries {
            if let Some(p) = b.percent {
                line2.push(BarItem::Device { kind: b.kind, percent: p as u32, charging: b.charging });
            }
        }
    }
    let lines: Vec<Vec<BarItem>> = [line1, line2].into_iter().filter(|l| !l.is_empty()).collect();
    let light_text = !taskbar_is_light();
    if lines.is_empty() {
        let _ = ShowWindow(hwnd, SW_HIDE);
    } else {
        present_overlay(hwnd, &lines, light_text);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    refresh_panel();

    // Ícone da bandeja: temperatura da CPU (ou GPU se não houver).
    let text = r.cpu.or(r.gpu).map_or("--".into(), |t| cfg.temp(t).to_string());
    if !tray_added || last_text.as_deref() != Some(&text) {
        if let Some(canvas) = Canvas::draw(&[text.clone()], 27, 700, (0, 0), Some((32, 32)), light_text, false) {
            if let Some(icon) = canvas.to_icon() {
                let nid = tray_data(hwnd, Some(icon), "Sensoryx");
                let ok = Shell_NotifyIconW(if tray_added { NIM_MODIFY } else { NIM_ADD }, &nid).as_bool();
                let _ = DestroyIcon(icon);
                APP.with(|a| {
                    if let Ok(mut g) = a.try_borrow_mut() {
                        if let Some(app) = g.as_mut() {
                            app.tray_added = ok;
                            app.tray_text = Some(text);
                        }
                    }
                });
            }
        }
    }
}

unsafe fn tray_data(hwnd: HWND, icon: Option<HICON>, tip: &str) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_MESSAGE | NIF_TIP | if icon.is_some() { NIF_ICON } else { NIF_MESSAGE },
        uCallbackMessage: WM_TRAY,
        hIcon: icon.unwrap_or_default(),
        ..Default::default()
    };
    for (i, c) in tip.encode_utf16().take(127).enumerate() {
        nid.szTip[i] = c;
    }
    nid
}

/// `true` se a barra de tarefas está no tema claro.
fn taskbar_is_light() -> bool {
    let mut v: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut c_void),
            Some(&mut size),
        )
    };
    r.is_ok() && v == 1
}

unsafe fn present_overlay(hwnd: HWND, lines: &[BarItemLine], light_text: bool) {
    // Geometria da barra: ancoramos à esquerda da área de notificação.
    let tray = FindWindowW(w!("Shell_TrayWnd"), None).unwrap_or_default();
    let mut tr = RECT::default();
    let have_tray = !tray.0.is_null() && GetWindowRect(tray, &mut tr).is_ok();
    if !have_tray {
        let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        tr = RECT { left: 0, top: sh - 48, right: sw, bottom: sh };
    }
    let tray_h = (tr.bottom - tr.top).max(1);
    let mut right = tr.right - 8;
    if have_tray {
        if let Ok(notify) = FindWindowExW(Some(tray), None, w!("TrayNotifyWnd"), None) {
            let mut nr = RECT::default();
            if GetWindowRect(notify, &mut nr).is_ok() && nr.left > tr.left {
                right = nr.left - 8;
            }
        }
    }

    let font_px = (tray_h * 27 / 100).max(11);
    let Some(canvas) = crate::panel::render_bar(lines, font_px, light_text) else { return };
    let x = right - canvas.w;
    let y = tr.top + (tray_h - canvas.h) / 2;
    blit_layered(hwnd, &canvas, x, y);
}

/// Mostra o bitmap (alfa por pixel) na janela em camadas, na posição dada.
unsafe fn blit_layered(hwnd: HWND, canvas: &Canvas, x: i32, y: i32) {
    let screen = GetDC(None);
    let pt = POINT { x, y };
    let size = SIZE { cx: canvas.w, cy: canvas.h };
    let src = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
    let _ = UpdateLayeredWindow(
        hwnd,
        Some(screen),
        Some(&pt as *const POINT),
        Some(&size as *const SIZE),
        Some(canvas.dc),
        Some(&src as *const POINT),
        COLORREF(0),
        Some(&blend as *const BLENDFUNCTION),
        ULW_ALPHA,
    );
    ReleaseDC(None, screen);
    // A barra de tarefas também é topmost: reafirma a ordem sem roubar o foco.
    let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
}

// ---------------------------------------------------------------- painel de benchmark

/// Alterna o painel (atalho global).
unsafe fn toggle_panel(hwnd: HWND) {
    let now_visible = APP.with(|a| {
        let mut g = a.try_borrow_mut().ok()?;
        let app = g.as_mut()?;
        app.panel_visible = !app.panel_visible;
        if !app.panel_visible {
            let _ = ShowWindow(app.panel, SW_HIDE);
        }
        Some(app.panel_visible)
    });
    if now_visible == Some(true) {
        refresh_panel();
    }
    let _ = hwnd;
}

/// Redesenha e posiciona o painel de benchmark (se estiver visível).
fn refresh_panel() {
    let Some((panel, settings, last, hist)) = APP.with(|a| {
        let g = a.try_borrow().ok()?;
        let app = g.as_ref()?;
        app.panel_visible.then(|| (app.panel, app.settings.clone(), app.last.clone(), app.fps_hist.clone()))
    }) else {
        return;
    };
    unsafe {
        let dpi = GetDpiForSystem().max(96) as f32;
        let scale = dpi / 96.0 * settings.overlay_scale as f32 / 100.0;
        let Some(canvas) = crate::panel::render(&crate::panel::model(&last, &hist, &settings), scale, settings.overlay_opacity) else {
            return;
        };

        // Monitor onde está o jogo (janela em primeiro plano).
        let mon = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let rc = if GetMonitorInfoW(mon, &mut mi).as_bool() { mi.rcMonitor } else { RECT { left: 0, top: 0, right: 1920, bottom: 1080 } };
        let margin = (20.0 * scale) as i32;
        let x = match settings.overlay_corner {
            Corner::TopLeft | Corner::BottomLeft => rc.left + margin,
            Corner::TopRight | Corner::BottomRight => rc.right - canvas.w - margin,
        };
        let y = match settings.overlay_corner {
            Corner::TopLeft | Corner::TopRight => rc.top + margin,
            Corner::BottomLeft | Corner::BottomRight => rc.bottom - canvas.h - margin,
        };
        blit_layered(panel, &canvas, x, y);
        let _ = ShowWindow(panel, SW_SHOWNOACTIVATE);
    }
}

/// Bitmap 32 bpp em memória onde o texto é desenhado com GDI e depois convertido em alfa.
pub(crate) struct Canvas {
    pub(crate) dc: HDC,
    pub(crate) bmp: HBITMAP,
    pub(crate) old: HGDIOBJ,
    pub(crate) w: i32,
    pub(crate) h: i32,
}

impl Canvas {
    /// Desenha `lines`. Com `fixed` o tamanho é fixo e o texto é centralizado (ícone);
    /// sem ele o bitmap se ajusta ao texto. `premul` = alfa pré-multiplicado (janela em camadas).
    fn draw(
        lines: &[String],
        font_px: i32,
        weight: i32,
        pad: (i32, i32),
        fixed: Option<(i32, i32)>,
        light_text: bool,
        premul: bool,
    ) -> Option<Self> {
        unsafe {
            let screen = GetDC(None);
            let dc = CreateCompatibleDC(Some(screen));
            let font = CreateFontW(
                -font_px,
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                ANTIALIASED_QUALITY,
                0,
                w!("Segoe UI"),
            );
            let old_font = SelectObject(dc, font.into());

            let wide: Vec<Vec<u16>> = lines.iter().map(|l| l.encode_utf16().collect()).collect();
            let (mut max_w, mut line_h) = (0, 0);
            for l in &wide {
                let mut sz = SIZE::default();
                let _ = GetTextExtentPoint32W(dc, l, &mut sz);
                max_w = max_w.max(sz.cx);
                line_h = line_h.max(sz.cy);
            }
            let total_h = line_h * lines.len() as i32;
            let (w, h) = fixed.unwrap_or((max_w + 2 * pad.0, total_h + 2 * pad.1));

            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut c_void = null_mut();
            let Ok(bmp) = CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
                SelectObject(dc, old_font);
                let _ = DeleteObject(font.into());
                let _ = DeleteDC(dc);
                ReleaseDC(None, screen);
                return None;
            };
            let old = SelectObject(dc, bmp.into());

            // Texto branco sobre fundo preto: o brilho vira a cobertura (alfa).
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, COLORREF(0x00FF_FFFF));
            let mut y = if fixed.is_some() { (h - total_h) / 2 } else { pad.1 };
            for l in &wide {
                let x = if fixed.is_some() {
                    let mut sz = SIZE::default();
                    let _ = GetTextExtentPoint32W(dc, l, &mut sz);
                    (w - sz.cx) / 2
                } else {
                    pad.0
                };
                let _ = TextOutW(dc, x, y, l);
                y += line_h;
            }
            let _ = GdiFlush();

            let (tr, tg, tb): (u32, u32, u32) = if light_text { (0xF2, 0xF2, 0xF2) } else { (0x20, 0x20, 0x20) };
            let px = std::slice::from_raw_parts_mut(bits as *mut u32, (w * h) as usize);
            for p in px.iter_mut() {
                let v = *p;
                let a = ((v >> 16) & 0xFF).max((v >> 8) & 0xFF).max(v & 0xFF);
                *p = if premul {
                    (a << 24) | ((tr * a / 255) << 16) | ((tg * a / 255) << 8) | (tb * a / 255)
                } else {
                    (a << 24) | (tr << 16) | (tg << 8) | tb
                };
            }

            SelectObject(dc, old_font);
            let _ = DeleteObject(font.into());
            ReleaseDC(None, screen);
            Some(Self { dc, bmp, old, w, h })
        }
    }

    /// Converte o bitmap em HICON (o chamador destrói com `DestroyIcon`).
    fn to_icon(&self) -> Option<HICON> {
        unsafe {
            SelectObject(self.dc, self.old); // o bitmap não pode estar selecionado num DC
            let mask = CreateBitmap(self.w, self.h, 1, 1, None);
            let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: self.bmp };
            let icon = CreateIconIndirect(&info).ok();
            let _ = DeleteObject(mask.into());
            icon
        }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bmp.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

#[allow(dead_code)]
fn _unused(_: PCWSTR) {}
