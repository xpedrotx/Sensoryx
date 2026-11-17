//! Configurações em `%APPDATA%\Sensoryx\settings.ini` (formato `chave = valor`).
//! A tela de configurações grava este arquivo e o app em execução o relê quando ele muda.

use std::{fs, path::PathBuf, time::SystemTime};

pub const MOD_ALT: u32 = 0x1;
pub const MOD_CONTROL: u32 = 0x2;
pub const MOD_SHIFT: u32 = 0x4;
pub const MOD_WIN: u32 = 0x8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    pub fn as_str(self) -> &'static str {
        match self {
            Corner::TopLeft => "top-left",
            Corner::TopRight => "top-right",
            Corner::BottomLeft => "bottom-left",
            Corner::BottomRight => "bottom-right",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    /// Atalho global que mostra/oculta o painel de benchmark, ex.: `Ctrl+Shift+F10`.
    pub hotkey: String,
    pub hotkey_mods: u32,
    pub hotkey_vk: u32,
    /// Canto da tela onde o painel aparece.
    pub overlay_corner: Corner,
    /// Escala do painel em % (50–300).
    pub overlay_scale: u32,
    /// Opacidade do fundo do painel em % (10–100).
    pub overlay_opacity: u32,
    // O que aparece no painel de benchmark.
    pub panel_graph: bool,
    pub panel_gpu: bool,
    pub panel_cpu: bool,
    pub panel_devices: bool,
    // O que aparece na barra de tarefas.
    pub bar_cpu: bool,
    pub bar_gpu: bool,
    pub bar_fps: bool,
    pub bar_devices: bool,
    /// Mostrar temperaturas em °F.
    pub fahrenheit: bool,
    /// Verificar atualizações ao abrir a tela de configurações (única chamada de rede do app).
    pub check_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let hotkey = "Ctrl+Shift+F10".to_string();
        let (hotkey_mods, hotkey_vk) = parse_hotkey(&hotkey).unwrap();
        Self {
            hotkey,
            hotkey_mods,
            hotkey_vk,
            overlay_corner: Corner::TopLeft,
            overlay_scale: 100,
            overlay_opacity: 90,
            panel_graph: true,
            panel_gpu: true,
            panel_cpu: true,
            panel_devices: true,
            bar_cpu: true,
            bar_gpu: true,
            bar_fps: true,
            bar_devices: true,
            fahrenheit: false,
            check_updates: false,
        }
    }
}

impl Settings {
    /// Converte °C para a unidade escolhida.
    pub fn temp(&self, c: u32) -> u32 {
        if self.fahrenheit { (c as f32 * 9.0 / 5.0 + 32.0).round() as u32 } else { c }
    }

    pub fn unit(&self) -> &'static str {
        if self.fahrenheit { "°F" } else { "°C" }
    }

    /// JSON para a tela de configurações (campos simples, sem escapes necessários).
    pub fn to_json(&self) -> String {
        format!(
            "{{\"hotkey\":\"{}\",\"overlay_corner\":\"{}\",\"overlay_scale\":{},\"overlay_opacity\":{},\
\"panel_graph\":{},\"panel_gpu\":{},\"panel_cpu\":{},\"panel_devices\":{},\
\"bar_cpu\":{},\"bar_gpu\":{},\"bar_fps\":{},\"bar_devices\":{},\"fahrenheit\":{},\"check_updates\":{}}}",
            self.hotkey,
            self.overlay_corner.as_str(),
            self.overlay_scale,
            self.overlay_opacity,
            self.panel_graph,
            self.panel_gpu,
            self.panel_cpu,
            self.panel_devices,
            self.bar_cpu,
            self.bar_gpu,
            self.bar_fps,
            self.bar_devices,
            self.fahrenheit,
            self.check_updates
        )
    }
}

fn path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("Sensoryx").join("settings.ini"))
}

/// Data de modificação do arquivo (o app em execução usa para detectar mudanças).
pub fn mtime() -> Option<SystemTime> {
    fs::metadata(path()?).ok()?.modified().ok()
}

/// Lê as configurações; cria o arquivo com os valores padrão se ele não existir.
/// Valores inválidos são ignorados (fica o padrão).
pub fn load() -> Settings {
    let mut s = Settings::default();
    let Some(p) = path() else { return s };
    match fs::read_to_string(&p) {
        Ok(t) => apply(&mut s, &t),
        Err(_) => {
            let _ = save(&s);
        }
    }
    s
}

/// Grava o arquivo completo, com comentários.
pub fn save(s: &Settings) -> std::io::Result<()> {
    let p = path().ok_or_else(|| std::io::Error::other("APPDATA indisponível"))?;
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir)?;
    }
    let b = |v: bool| if v { "true" } else { "false" };
    let text = format!(
        "\
; Sensoryx - configuracoes (tambem editaveis pela tela de configuracoes)
; Atalho para mostrar/ocultar o painel de benchmark sobre o jogo.
; Modificadores: Ctrl, Alt, Shift, Win. Teclas: A-Z, 0-9, F1-F24, Insert, Delete, Home, End,
; PageUp, PageDown, Pause, ScrollLock, Space, Num0-Num9.
hotkey = {}
; Canto do painel: top-left, top-right, bottom-left, bottom-right
overlay_corner = {}
; Escala do painel em % (50 a 300)
overlay_scale = {}
; Opacidade do fundo em % (10 a 100)
overlay_opacity = {}
; O que aparece no painel de benchmark
panel_graph = {}
panel_gpu = {}
panel_cpu = {}
panel_devices = {}
; O que aparece na barra de tarefas
bar_cpu = {}
bar_gpu = {}
bar_fps = {}
bar_devices = {}
; Temperaturas em Fahrenheit
fahrenheit = {}
; Verificar atualizacoes ao abrir a tela de configuracoes (unica chamada de rede do app)
check_updates = {}
",
        s.hotkey,
        s.overlay_corner.as_str(),
        s.overlay_scale,
        s.overlay_opacity,
        b(s.panel_graph),
        b(s.panel_gpu),
        b(s.panel_cpu),
        b(s.panel_devices),
        b(s.bar_cpu),
        b(s.bar_gpu),
        b(s.bar_fps),
        b(s.bar_devices),
        b(s.fahrenheit),
        b(s.check_updates),
    );
    fs::write(p, text)
}

/// Aplica uma chave. Devolve `false` se a chave ou o valor forem inválidos.
pub fn set(s: &mut Settings, key: &str, v: &str) -> bool {
    let v = v.trim();
    let flag = |t: &str| match t.to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    };
    let set_flag = |slot: &mut bool| match flag(v) {
        Some(b) => {
            *slot = b;
            true
        }
        None => false,
    };
    match key.trim().to_lowercase().as_str() {
        "hotkey" => match parse_hotkey(v) {
            Some((m, vk)) => {
                s.hotkey = v.to_string();
                s.hotkey_mods = m;
                s.hotkey_vk = vk;
                true
            }
            None => false,
        },
        "overlay_corner" => {
            let c = match v.to_lowercase().as_str() {
                "top-left" => Corner::TopLeft,
                "top-right" => Corner::TopRight,
                "bottom-left" => Corner::BottomLeft,
                "bottom-right" => Corner::BottomRight,
                _ => return false,
            };
            s.overlay_corner = c;
            true
        }
        "overlay_scale" => v.parse::<u32>().map(|n| s.overlay_scale = n.clamp(50, 300)).is_ok(),
        "overlay_opacity" => v.parse::<u32>().map(|n| s.overlay_opacity = n.clamp(10, 100)).is_ok(),
        "panel_graph" => set_flag(&mut s.panel_graph),
        "panel_gpu" => set_flag(&mut s.panel_gpu),
        "panel_cpu" => set_flag(&mut s.panel_cpu),
        "panel_devices" => set_flag(&mut s.panel_devices),
        "bar_cpu" => set_flag(&mut s.bar_cpu),
        "bar_gpu" => set_flag(&mut s.bar_gpu),
        "bar_fps" => set_flag(&mut s.bar_fps),
        "bar_devices" => set_flag(&mut s.bar_devices),
        "fahrenheit" => set_flag(&mut s.fahrenheit),
        "check_updates" => set_flag(&mut s.check_updates),
        _ => false,
    }
}

fn apply(s: &mut Settings, text: &str) {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let _ = set(s, k, v);
        }
    }
}

/// Converte `"Ctrl+Shift+F10"` em (modificadores, código de tecla virtual).
pub fn parse_hotkey(text: &str) -> Option<(u32, u32)> {
    let mut mods = 0;
    let mut vk = None;
    for part in text.split('+').map(|p| p.trim()) {
        match part.to_lowercase().as_str() {
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "alt" => mods |= MOD_ALT,
            "shift" => mods |= MOD_SHIFT,
            "win" | "windows" => mods |= MOD_WIN,
            key => {
                if vk.is_some() {
                    return None; // só uma tecla principal
                }
                vk = Some(key_code(key)?);
            }
        }
    }
    // Exige pelo menos um modificador para não roubar uma tecla "solta" do jogo.
    (mods != 0).then_some(())?;
    Some((mods, vk?))
}

fn key_code(key: &str) -> Option<u32> {
    let b = key.as_bytes();
    if b.len() == 1 && b[0].is_ascii_alphanumeric() {
        return Some(b[0].to_ascii_uppercase() as u32);
    }
    if let Some(n) = key.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        if (1..=24).contains(&n) {
            return Some(0x70 + n - 1);
        }
    }
    if let Some(n) = key.strip_prefix("num").and_then(|n| n.parse::<u32>().ok()) {
        if n <= 9 {
            return Some(0x60 + n);
        }
    }
    Some(match key {
        "insert" => 0x2D,
        "delete" => 0x2E,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "pause" => 0x13,
        "scrolllock" => 0x91,
        "space" => 0x20,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_parsing() {
        assert_eq!(parse_hotkey("Ctrl+Shift+F10"), Some((MOD_CONTROL | MOD_SHIFT, 0x79)));
        assert_eq!(parse_hotkey("alt + b"), Some((MOD_ALT, b'B' as u32)));
        assert_eq!(parse_hotkey("Win+Num5"), Some((MOD_WIN, 0x65)));
        assert_eq!(parse_hotkey("F10"), None); // sem modificador
        assert_eq!(parse_hotkey("Ctrl+A+B"), None); // duas teclas
        assert_eq!(parse_hotkey("Ctrl+Banana"), None);
        assert_eq!(parse_hotkey("Ctrl+F25"), None);
    }

    #[test]
    fn invalid_values_keep_defaults() {
        let mut s = Settings::default();
        apply(&mut s, "hotkey = Banana\noverlay_scale = 9999\noverlay_opacity = abc\noverlay_corner = meio\nbar_cpu = talvez");
        assert_eq!(s.hotkey, "Ctrl+Shift+F10");
        assert_eq!(s.overlay_scale, 300); // limitado ao máximo
        assert_eq!(s.overlay_opacity, 90);
        assert_eq!(s.overlay_corner, Corner::TopLeft);
        assert!(s.bar_cpu);
    }

    #[test]
    fn valid_file() {
        let mut s = Settings::default();
        apply(&mut s, "; c\nhotkey = Alt+F9\noverlay_corner = Bottom-Right\noverlay_scale = 150\noverlay_opacity = 60\nbar_gpu = false\nfahrenheit = on");
        assert_eq!((s.hotkey_mods, s.hotkey_vk), (MOD_ALT, 0x78));
        assert_eq!(s.overlay_corner, Corner::BottomRight);
        assert_eq!((s.overlay_scale, s.overlay_opacity), (150, 60));
        assert!(!s.bar_gpu && s.fahrenheit);
    }

    #[test]
    fn fahrenheit_conversion() {
        let mut s = Settings::default();
        assert_eq!(s.temp(40), 40);
        s.fahrenheit = true;
        assert_eq!((s.temp(0), s.temp(40), s.temp(100)), (32, 104, 212));
        assert_eq!(s.unit(), "°F");
    }

    #[test]
    fn json_roundtrip_shape() {
        let j = Settings::default().to_json();
        assert!(j.starts_with("{\"hotkey\":\"Ctrl+Shift+F10\""));
        assert!(j.contains("\"fahrenheit\":false") && j.ends_with('}'));
    }
}
