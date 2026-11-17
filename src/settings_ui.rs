//! Tela de configurações: janela com WebView2 (HTML/CSS embutido), executada em um processo
//! separado (`sensoryx.exe --settings`) para não pesar na memória do app principal.
//! Ela grava `settings.ini`; o app em execução percebe a mudança e aplica na hora.

use crate::{autostart, settings, update};
use std::{path::PathBuf, sync::mpsc};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
use windows::{
    core::w,
    Win32::{
        Foundation::{GetLastError, ERROR_ALREADY_EXISTS},
        System::Threading::CreateMutexW,
        UI::WindowsAndMessaging::{
            FindWindowW, MessageBoxW, SetForegroundWindow, ShowWindow, MB_ICONINFORMATION, MB_OK, SW_RESTORE,
        },
    },
};
use wry::{WebContext, WebViewBuilder};

const TITLE: &str = "Sensoryx — Configurações";
const HTML: &str = include_str!("../ui/settings.html");

enum UserEvent {
    Eval(String),
}

/// JSON-escapa uma string simples para inserir em um script.
fn js_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => {}
            c if (c as u32) < 0x20 => {}
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Sensoryx").join("webview")
}

/// Garante uma única janela de configurações; se já houver uma, traz ela para a frente.
fn already_open() -> bool {
    unsafe {
        let _ = CreateMutexW(None, false, w!("Local\\SensoryxSettingsWindow"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let title: Vec<u16> = TITLE.encode_utf16().chain(Some(0)).collect();
            if let Ok(h) = FindWindowW(None, windows::core::PCWSTR(title.as_ptr())) {
                let _ = ShowWindow(h, SW_RESTORE);
                let _ = SetForegroundWindow(h);
            }
            return true;
        }
    }
    false
}

pub fn run() {
    if already_open() {
        return;
    }
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title(TITLE)
        .with_inner_size(LogicalSize::new(980.0, 700.0))
        .with_min_inner_size(LogicalSize::new(860.0, 600.0))
        .build(&event_loop)
        .expect("janela");

    // Progresso das atualizações (thread de download -> tela).
    let (ptx, prx) = mpsc::channel::<update::Progress>();
    {
        let proxy = proxy.clone();
        std::thread::spawn(move || {
            while let Ok(p) = prx.recv() {
                let js = match p {
                    update::Progress::Percent(n) => format!("sensoryx.progress({n})"),
                    update::Progress::Verifying => "sensoryx.verifying()".into(),
                    update::Progress::Installing => "sensoryx.installing()".into(),
                    update::Progress::Failed(m) => format!("sensoryx.failed({})", js_str(&m)),
                };
                if proxy.send_event(UserEvent::Eval(js)).is_err() {
                    break;
                }
            }
        });
    }

    let ipc_proxy = proxy.clone();
    let mut ctx = WebContext::new(Some(data_dir()));
    let webview = WebViewBuilder::new_with_web_context(&mut ctx)
        .with_initialization_script(
            "window.addEventListener('error',function(e){window.ipc.postMessage('jserr:'+e.message+' @'+e.lineno)});\
             window.ipc.postMessage('init-script-ok');",
        )
        .with_html(HTML)
        .with_ipc_handler(move |req| {
            let msg = req.body().to_string();
            let proxy = ipc_proxy.clone();
            let ptx = ptx.clone();
            #[cfg(debug_assertions)]
            {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(std::env::temp_dir().join("sensoryx-ui.log")) {
                    let _ = writeln!(f, "ipc: {msg}");
                }
            }
            if msg == "ready" {
                let s = settings::load();
                let js = format!(
                    "sensoryx.init({},{{version:{},repo:{},autostart:{}}})",
                    s.to_json(),
                    js_str(update::VERSION),
                    js_str(update::UPDATE_REPO),
                    autostart::is_enabled()
                );
                let _ = proxy.send_event(UserEvent::Eval(js));
            } else if let Some(kv) = msg.strip_prefix("set:") {
                if let Some((k, v)) = kv.split_once('=') {
                    let mut s = settings::load();
                    if settings::set(&mut s, k, v) {
                        let _ = settings::save(&s);
                    }
                }
            } else if let Some(on) = msg.strip_prefix("autostart:") {
                let on = on == "on";
                std::thread::spawn(move || {
                    let state = autostart::set(on);
                    let _ = proxy.send_event(UserEvent::Eval(format!("sensoryx.autostart({state})")));
                });
            } else if let Some(args) = msg.strip_prefix("update:install|") {
                let parts: Vec<String> = args.split('|').map(str::to_string).collect();
                if parts.len() == 3 {
                    std::thread::spawn(move || {
                        let size = parts[2].parse::<u64>().unwrap_or(0);
                        update::download_and_install(&parts[0], &parts[1], size, &ptx);
                    });
                }
            }
        })
        .build(&window);
    let webview = match webview {
        Ok(w) => w,
        Err(_) => {
            // Sem o WebView2 Runtime (Windows 10 antigo): explica em vez de fechar sem aviso.
            unsafe {
                MessageBoxW(
                    None,
                    w!("A tela de configurações precisa do Microsoft Edge WebView2 Runtime, que não foi encontrado.\n\nInstale em:\nhttps://developer.microsoft.com/microsoft-edge/webview2/\n\nO Sensoryx continua funcionando normalmente na barra de tarefas."),
                    w!("Sensoryx"),
                    MB_OK | MB_ICONINFORMATION,
                );
            }
            return;
        }
    };

    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Eval(js)) => {
                let _ = webview.evaluate_script(&js);
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => *flow = ControlFlow::Exit,
            _ => {}
        }
    });
}
