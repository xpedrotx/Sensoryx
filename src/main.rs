// No release não abre console; em debug mantém para ver mensagens.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod cpu;
mod fps;
mod gpu;
mod hidpp;
mod panel;
mod pawnio;
mod settings;
mod settings_ui;
mod sys;
mod ui;
mod update;

use std::sync::{Arc, Mutex};

fn main() {
    if std::env::args().any(|a| a == "--settings") {
        settings_ui::run();
        return;
    }

    if std::env::args().any(|a| a == "--probe") {
        let t = std::time::Instant::now();
        let found = hidpp::scan();
        println!("{} dispositivo(s) em {:?}", found.len(), t.elapsed());
        for b in found {
            println!("  {:?} {} -> {:?}% carregando={}", b.kind, b.name, b.percent, b.charging);
        }
        return;
    }

    #[cfg(debug_assertions)]
    if std::env::args().any(|a| a == "--cpu") {
        let dir = std::env::current_exe().unwrap().parent().unwrap().join("modules");
        cpu::diagnose(&dir);
        return;
    }

    if std::env::args().any(|a| a == "--fps") {
        println!("sessão ETW: {}", fps::start());
        for _ in 0..6 {
            std::thread::sleep(std::time::Duration::from_secs(1));
            println!("{:?}", fps::all_fps());
        }
        fps::stop();
        return;
    }

    let fps_ok = fps::start();

    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default();

    let gpu = gpu::NvmlGpu::open();
    let cpu = match cpu::Cpu::open(&exe_dir.join("modules")) {
        Ok(c) => Some(c),
        Err(e) => {
            #[cfg(debug_assertions)]
            eprintln!("CPU: {e}");
            let _ = e;
            None
        }
    };

    // Bateria dos periféricos: varredura em segundo plano (pode demorar se um dispositivo dormir).
    let batteries = Arc::new(Mutex::new(Vec::<hidpp::Battery>::new()));
    {
        let batteries = Arc::clone(&batteries);
        std::thread::spawn(move || loop {
            let found = hidpp::scan();
            *batteries.lock().unwrap() = found;
            std::thread::sleep(std::time::Duration::from_secs(60));
        });
    }

    let cpu_name = cpu::brand_name();
    let mut cpu_load = sys::CpuLoad::default();

    // `--demo` (só em debug): dados simulados para avaliar o visual sem precisar de um jogo.
    #[cfg(debug_assertions)]
    let demo = std::env::args().any(|a| a == "--demo");
    #[cfg(not(debug_assertions))]
    let demo = false;
    let t0 = std::time::Instant::now();

    ui::run(settings::load(), move || {
        let g = gpu.as_ref().map(|g| g.stats()).unwrap_or_default();
        let mut r = ui::Readings {
            cpu: cpu.as_ref().and_then(|c| c.temperature()),
            gpu: g.temp,
            fps: if fps_ok { fps::foreground_fps() } else { None },
            batteries: batteries.lock().unwrap().clone(),
            gpu_name: g.name,
            cpu_name: cpu_name.clone(),
            gpu_usage: g.usage,
            gpu_power_w: g.power_w,
            gpu_vram_mb: g.vram_used_mb,
            gpu_core_mhz: g.core_mhz,
            gpu_mem_mhz: g.mem_mhz,
            cpu_usage: cpu_load.usage(),
            ram_mb: sys::ram_used_mb(),
        };
        if demo {
            let s = t0.elapsed().as_secs_f32();
            r.fps = Some((98.0 + 14.0 * (s * 0.7).sin() + 5.0 * (s * 2.3).sin()) as u32);
            r.gpu_usage = Some(96);
            r.gpu = Some(71);
            r.gpu_power_w = Some(156);
            r.gpu_vram_mb = Some(5369);
            r.gpu_core_mhz = Some(2610);
            r.gpu_mem_mhz = Some(14001);
            r.cpu = Some(78);
            r.cpu_usage = Some(34);
            r.batteries = vec![
                hidpp::Battery { name: "G Pro".into(), kind: hidpp::Kind::Mouse, percent: Some(82), charging: false },
                hidpp::Battery { name: "G733".into(), kind: hidpp::Kind::Headset, percent: Some(18), charging: false },
            ];
        }
        r
    });
    fps::stop();
}

