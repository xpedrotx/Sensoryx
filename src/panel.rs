//! Painel de benchmark sobre o jogo: cartão de vidro escuro, FPS em destaque com gráfico,
//! barras de uso e métricas em grade. Tudo desenhado em software, com bordas suavizadas
//! (funções de distância) sobre um bitmap com alfa pré-multiplicado.

use crate::hidpp::Kind;
use crate::settings::Settings;
use crate::ui::{Canvas, Readings};
use std::{ffi::c_void, mem::size_of, ptr::null_mut};
use windows::{
    core::w,
    Win32::{Foundation::*, Graphics::Gdi::*},
};

pub type Rgb = (u32, u32, u32);

const TEXT: Rgb = (0xF3, 0xF4, 0xF8);
const MUTED: Rgb = (0x8E, 0x93, 0xA6);
const GOOD: Rgb = (0x5B, 0xE5, 0x84);
const WARN: Rgb = (0xFF, 0xC2, 0x4D);
const HOT: Rgb = (0xFF, 0x92, 0x48);
const BAD: Rgb = (0xFF, 0x5C, 0x5C);
const ICON_COLOR: Rgb = (0xB4, 0xB9, 0xCB);
const GPU_ACCENT: Rgb = (0x7E, 0xD9, 0x57);
const CPU_ACCENT: Rgb = (0x5A, 0xA9, 0xFF);
const DEV_ACCENT: Rgb = (0xB2, 0x8C, 0xFF);

const WIDTH: f32 = 268.0;
const PAD: f32 = 18.0;

pub struct Cell {
    pub label: String,
    pub value: String,
    pub unit: String,
    pub color: Rgb,
    /// Quando presente, o ícone desenhado substitui o texto do rótulo.
    pub icon: Option<Kind>,
}

pub struct Section {
    pub title: String,
    pub accent: Rgb,
    pub usage: Option<u32>,
    pub cells: Vec<Cell>,
}

pub struct Model {
    pub fps: Option<u32>,
    pub hist: Vec<u32>,
    pub show_graph: bool,
    pub sections: Vec<Section>,
}

// ------------------------------------------------------------------ modelo

fn temp_color(t: u32) -> Rgb {
    match t {
        0..=59 => (0x7F, 0xE3, 0xA0),
        60..=74 => WARN,
        75..=84 => HOT,
        _ => BAD,
    }
}

fn level_color(pct: u32) -> Rgb {
    match pct {
        0..=15 => BAD,
        16..=35 => WARN,
        _ => GOOD,
    }
}

fn fmt_mem(mb: u32) -> (String, &'static str) {
    if mb >= 1024 {
        (format!("{:.1}", mb as f32 / 1024.0), "GB")
    } else {
        (mb.to_string(), "MB")
    }
}

fn fmt_clock(mhz: u32) -> (String, &'static str) {
    if mhz >= 1000 {
        (format!("{:.2}", mhz as f32 / 1000.0), "GHz")
    } else {
        (mhz.to_string(), "MHz")
    }
}

fn short_gpu(name: &str) -> String {
    let s = name.trim_start_matches("NVIDIA ").trim_start_matches("GeForce ").trim();
    if s.is_empty() { "GPU".into() } else { s.to_string() }
}

/// "12th Gen Intel(R) Core(TM) i5-12400F" -> "i5-12400F"; "AMD Ryzen 5 5600X 6-Core Processor" -> "Ryzen 5 5600X".
fn short_cpu(brand: &str) -> String {
    if let Some(t) = brand.split_whitespace().find(|t| {
        let b = t.as_bytes();
        b.len() > 3 && b[0] == b'i' && b[1].is_ascii_digit() && b[2] == b'-'
    }) {
        return t.to_string();
    }
    let s = brand.trim_start_matches("AMD ").trim();
    let s = s.split(" Processor").next().unwrap_or(s);
    let s = s.split_whitespace().filter(|t| !t.ends_with("-Core")).collect::<Vec<_>>().join(" ");
    if s.is_empty() { "CPU".into() } else { s }
}

pub fn model(r: &Readings, hist: &[u32], cfg: &Settings) -> Model {
    let cell = |label: &str, value: String, unit: &str, color: Rgb| Cell { label: label.into(), value, unit: unit.into(), color, icon: None };
    let mut sections = Vec::new();

    if cfg.panel_gpu && (!r.gpu_name.is_empty() || r.gpu.is_some()) {
        let mut cells = Vec::new();
        if let Some(t) = r.gpu { cells.push(cell("TEMP", cfg.temp(t).to_string(), cfg.unit(), temp_color(t))) }
        if let Some(v) = r.gpu_power_w { cells.push(cell("POWER", v.to_string(), "W", TEXT)) }
        if let Some(v) = r.gpu_vram_mb { let (n, u) = fmt_mem(v); cells.push(cell("VRAM", n, u, TEXT)) }
        if let Some(v) = r.gpu_core_mhz { let (n, u) = fmt_clock(v); cells.push(cell("CORE", n, u, TEXT)) }
        if let Some(v) = r.gpu_mem_mhz { let (n, u) = fmt_clock(v); cells.push(cell("MEMORY", n, u, TEXT)) }
        sections.push(Section { title: short_gpu(&r.gpu_name), accent: GPU_ACCENT, usage: r.gpu_usage, cells });
    }

    if cfg.panel_cpu {
        let mut cells = Vec::new();
        if let Some(t) = r.cpu { cells.push(cell("TEMP", cfg.temp(t).to_string(), cfg.unit(), temp_color(t))) }
        if let Some(v) = r.ram_mb { let (n, u) = fmt_mem(v); cells.push(cell("RAM", n, u, TEXT)) }
        sections.push(Section {
            title: if r.cpu_name.is_empty() { "CPU".into() } else { short_cpu(&r.cpu_name) },
            accent: CPU_ACCENT,
            usage: r.cpu_usage,
            cells,
        });
    }

    let devs: Vec<Cell> = r
        .batteries
        .iter()
        .filter(|_| cfg.panel_devices)
        .filter_map(|b| {
            let p = b.percent? as u32;
            let mut c = cell(&b.kind.label().to_uppercase(), p.to_string(), "%", if b.charging { CPU_ACCENT } else { level_color(p) });
            c.icon = Some(b.kind);
            Some(c)
        })
        .collect();
    if !devs.is_empty() {
        sections.push(Section { title: "Periféricos".into(), accent: DEV_ACCENT, usage: None, cells: devs });
    }

    Model { fps: r.fps, hist: hist.to_vec(), show_graph: cfg.panel_graph, sections }
}

// ------------------------------------------------------------------ pintura

#[derive(Clone, Copy)]
enum FontKind {
    Hero = 0,
    Title = 1,
    Label = 2,
    Value = 3,
    Unit = 4,
}

#[derive(Clone, Copy)]
struct Font {
    h: HFONT,
    ascent: i32,
    height: i32,
}

struct Painter {
    dc: HDC,
    out: *mut u32,
    cov: *mut u32,
    w: i32,
    h: i32,
    fonts: Vec<Font>,
    scale: f32,
    /// Passagem só para medir a altura: nada é desenhado.
    dry: bool,
}

fn over(d: u32, rgb: Rgb, a: f32) -> u32 {
    let a = a.clamp(0.0, 1.0);
    if a <= 0.0 {
        return d;
    }
    let ia = 1.0 - a;
    let ch = |shift: u32, src: u32| -> u32 {
        let dv = ((d >> shift) & 0xFF) as f32;
        ((src as f32 * a + dv * ia) + 0.5).min(255.0) as u32
    };
    let na = ((a * 255.0 + ((d >> 24) as f32) * ia) + 0.5).min(255.0) as u32;
    (na << 24) | (ch(16, rgb.0) << 16) | (ch(8, rgb.1) << 8) | ch(0, rgb.2)
}

impl Painter {
    fn px(&self, v: f32) -> i32 {
        (v * self.scale).round() as i32
    }

    fn shape(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, rgb: Rgb, alpha: f32, cov: impl Fn(f32, f32) -> f32) {
        if self.dry {
            return;
        }
        for y in y0.max(0)..y1.min(self.h) {
            for x in x0.max(0)..x1.min(self.w) {
                let c = cov(x as f32 + 0.5, y as f32 + 0.5) * alpha;
                if c > 0.0 {
                    unsafe {
                        let p = self.out.add((y * self.w + x) as usize);
                        *p = over(*p, rgb, c);
                    }
                }
            }
        }
    }

    fn rrect(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, rgb: Rgb, alpha: f32) {
        let (cx, cy) = (x as f32 + w as f32 / 2.0, y as f32 + h as f32 / 2.0);
        let (hx, hy, r) = (w as f32 / 2.0, h as f32 / 2.0, r as f32);
        self.shape(x - 1, y - 1, x + w + 1, y + h + 1, rgb, alpha, move |px, py| {
            let qx = (px - cx).abs() - (hx - r);
            let qy = (py - cy).abs() - (hy - r);
            let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
            (0.5 - d).clamp(0.0, 1.0)
        });
    }

    fn circle(&mut self, cx: i32, cy: i32, r: i32, rgb: Rgb, alpha: f32) {
        self.rrect(cx - r, cy - r, 2 * r, 2 * r, r, rgb, alpha);
    }

    fn divider(&mut self, x: i32, y: i32, w: i32) {
        self.rrect(x, y, w, 1.max(self.px(1.0)), 0, (255, 255, 255), 0.08);
    }

    /// Fundo do cartão: gradiente vertical, cantos arredondados e borda de 1 px.
    fn card(&mut self, opacity_pct: u32) {
        let a = opacity_pct.clamp(10, 100) as f32 / 100.0;
        let r = self.px(18.0) as f32;
        let (w, h) = (self.w as f32, self.h as f32);
        let (cx, cy) = (w / 2.0, h / 2.0);
        for y in 0..self.h {
            let t = y as f32 / h;
            let col = (
                (0x1D as f32 + (0x10 as f32 - 0x1D as f32) * t) as u32,
                (0x1D as f32 + (0x10 as f32 - 0x1D as f32) * t) as u32,
                (0x28 as f32 + (0x16 as f32 - 0x28 as f32) * t) as u32,
            );
            for x in 0..self.w {
                let qx = (x as f32 + 0.5 - cx).abs() - (cx - r);
                let qy = (y as f32 + 0.5 - cy).abs() - (cy - r);
                let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
                let inside = (0.5 - d).clamp(0.0, 1.0);
                if inside <= 0.0 {
                    continue;
                }
                let border = (inside - (0.5 - (d + 1.0)).clamp(0.0, 1.0)).max(0.0);
                unsafe {
                    let p = self.out.add((y * self.w + x) as usize);
                    *p = over(0, col, a * inside);
                    *p = over(*p, (255, 255, 255), 0.13 * border);
                }
            }
        }
    }

    fn measure(&self, kind: FontKind, s: &str) -> i32 {
        if self.dry {
            return 0;
        }
        unsafe {
            SelectObject(self.dc, self.fonts[kind as usize].h.into());
            let u: Vec<u16> = s.encode_utf16().collect();
            let mut sz = SIZE::default();
            let _ = GetTextExtentPoint32W(self.dc, &u, &mut sz);
            sz.cx
        }
    }

    /// Desenha texto com a linha de base em `baseline`; devolve a largura.
    fn text(&mut self, x: i32, baseline: i32, s: &str, kind: FontKind, color: Rgb) -> i32 {
        if self.dry || s.is_empty() {
            return 0;
        }
        let (ascent, height) = (self.fonts[kind as usize].ascent, self.fonts[kind as usize].height);
        let width = self.measure(kind, s);
        let top = baseline - ascent;
        unsafe {
            let u: Vec<u16> = s.encode_utf16().collect();
            let _ = TextOutW(self.dc, x, top, &u);
            let _ = GdiFlush();
            for yy in top.max(0)..(top + height).min(self.h) {
                for xx in x.max(0)..(x + width + 2).min(self.w) {
                    let i = (yy * self.w + xx) as usize;
                    let v = *self.cov.add(i);
                    let c = ((v >> 16) & 0xFF).max((v >> 8) & 0xFF).max(v & 0xFF);
                    if c == 0 {
                        continue;
                    }
                    *self.cov.add(i) = 0;
                    let p = self.out.add(i);
                    *p = over(*p, color, c as f32 / 255.0);
                }
            }
        }
        width
    }

    fn text_right(&mut self, right: i32, baseline: i32, s: &str, kind: FontKind, color: Rgb) -> i32 {
        let w = self.measure(kind, s);
        self.text(right - w, baseline, s, kind, color)
    }

    /// Gráfico de linha com área suave (últimos segundos de FPS).
    fn sparkline(&mut self, x: i32, y: i32, w: i32, h: i32, hist: &[u32], color: Rgb) {
        if self.dry {
            return;
        }
        if hist.len() < 2 {
            self.rrect(x, y + h / 2, w, 1.max(self.px(1.0)), 0, (255, 255, 255), 0.10);
            return;
        }
        let (mn, mx) = (*hist.iter().min().unwrap() as f32, *hist.iter().max().unwrap() as f32);
        let lo = mn * 0.8;
        let hi = (mx * 1.1).max(lo + 20.0);
        let n = hist.len();
        let pad = self.px(3.0) as f32;
        let ys: Vec<f32> = (0..w)
            .map(|xi| {
                let t = xi as f32 / (w - 1).max(1) as f32 * (n - 1) as f32;
                let i = (t.floor() as usize).min(n - 2);
                let f = t - i as f32;
                let v = hist[i] as f32 * (1.0 - f) + hist[i + 1] as f32 * f;
                y as f32 + h as f32 - pad - (v - lo) / (hi - lo) * (h as f32 - 2.0 * pad)
            })
            .collect();
        let thick = 1.8 * self.scale;
        let (ysr, hf) = (&ys, h as f32);
        self.shape(x, y, x + w, y + h, color, 1.0, |px, py| {
            let xi = ((px - x as f32) as usize).min(w as usize - 1);
            let yl = ysr[xi];
            let slope = (ysr[(xi + 1).min(w as usize - 1)] - ysr[xi.saturating_sub(1)]) / 2.0;
            let dist = (py - yl).abs() / (1.0 + slope * slope).sqrt();
            let line = (thick / 2.0 - dist + 0.5).clamp(0.0, 1.0);
            let area = if py > yl { 0.20 * (1.0 - (py - yl) / hf).clamp(0.0, 1.0) } else { 0.0 };
            line.max(area)
        });
        let r = self.px(3.0);
        self.circle(x + w - 1, ys[w as usize - 1].round() as i32, r, color, 1.0);
    }
}

// ------------------------------------------------------------------ ícones

/// Distância com sinal até um retângulo arredondado (negativa dentro).
fn rr_d(px: f32, py: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - (hx - r);
    let qy = (py - cy).abs() - (hy - r);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

fn cvg(d: f32) -> f32 {
    (0.5 - d).clamp(0.0, 1.0)
}

/// Largura ocupada por um ícone de tamanho `s`.
pub fn icon_width(kind: Kind, s: f32) -> f32 {
    match kind {
        Kind::Mouse => 0.62 * s,
        Kind::Headset => 1.0 * s,
        Kind::Keyboard => 1.3 * s,
        Kind::Other => 0.62 * s,
    }
}

impl Painter {
    fn shape_f(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, rgb: Rgb, alpha: f32, cov: impl Fn(f32, f32) -> f32) {
        self.shape(x0.floor() as i32 - 1, y0.floor() as i32 - 1, x1.ceil() as i32 + 1, y1.ceil() as i32 + 1, rgb, alpha, cov);
    }

    fn fill_rr(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, rgb: Rgb) {
        let (cx, cy, hx, hy) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
        self.shape_f(x, y, x + w, y + h, rgb, 1.0, move |px, py| cvg(rr_d(px, py, cx, cy, hx, hy, r)));
    }

    /// Contorno (traço) de um retângulo arredondado.
    fn ring_rr(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, stroke: f32, rgb: Rgb) {
        let (cx, cy, hx, hy) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
        self.shape_f(x, y, x + w, y + h, rgb, 1.0, move |px, py| {
            let d = rr_d(px, py, cx, cy, hx, hy, r);
            (cvg(d) - cvg(d + stroke)).max(0.0)
        });
    }

    /// Ícone de periférico em traço fino, centrado em (`cx`, `cy`) e com altura ~`s`.
    /// `charging` acrescenta um ponto verde no canto.
    fn icon(&mut self, kind: Kind, cx: f32, cy: f32, s: f32, rgb: Rgb, charging: bool) {
        if self.dry {
            return;
        }
        let sw = (s * 0.1).max(1.25);
        match kind {
            Kind::Mouse => {
                let (w, h) = (0.62 * s, 0.98 * s);
                self.ring_rr(cx - w / 2.0, cy - h / 2.0, w, h, w / 2.0, sw, rgb);
                self.fill_rr(cx - sw / 2.0, cy - 0.40 * s, sw, 0.22 * s, sw / 2.0, rgb); // roda
                self.fill_rr(cx - w / 2.0 + sw, cy - 0.08 * s, w - 2.0 * sw, sw, sw / 2.0, rgb); // botões
            }
            Kind::Headset => {
                let r = 0.40 * s;
                // Arco do headband: só a metade de cima de um anel.
                self.shape_f(cx - r - sw, cy - r - sw, cx + r + sw, cy + sw, rgb, 1.0, move |px, py| {
                    let d = ((px - cx).hypot(py - cy) - r).abs() - sw / 2.0;
                    cvg(d) * (cy - py + 0.5).clamp(0.0, 1.0)
                });
                let (cw, ch) = (0.2 * s, 0.42 * s);
                self.fill_rr(cx - r - cw / 2.0, cy - 0.02 * s, cw, ch, cw * 0.38, rgb);
                self.fill_rr(cx + r - cw / 2.0, cy - 0.02 * s, cw, ch, cw * 0.38, rgb);
            }
            Kind::Keyboard => {
                let (w, h) = (1.3 * s, 0.74 * s);
                self.ring_rr(cx - w / 2.0, cy - h / 2.0, w, h, 0.14 * s, sw, rgb);
                let k = 0.1 * s;
                for row in 0..2 {
                    for i in 0..4 {
                        self.fill_rr(cx - 0.42 * s + i as f32 * 0.28 * s, cy - 0.24 * s + row as f32 * 0.16 * s, k, k * 0.9, 0.02 * s, rgb);
                    }
                }
                self.fill_rr(cx - 0.34 * s, cy + 0.12 * s, 0.68 * s, sw, sw / 2.0, rgb); // barra de espaço
            }
            Kind::Other => {
                let d = 0.62 * s;
                self.ring_rr(cx - d / 2.0, cy - d / 2.0, d, d, d / 2.0, sw, rgb);
            }
        }
        if charging {
            let (d, w) = (0.26 * s, icon_width(kind, s));
            self.fill_rr(cx + w / 2.0 - d * 0.6, cy - 0.5 * s - d * 0.2, d, d, d / 2.0, GOOD);
        }
    }
}

/// Percorre o layout; devolve a altura total. Com `p.dry` só mede.
fn draw(p: &mut Painter, m: &Model) -> i32 {
    let sc = p.scale;
    let px = |v: f32| (v * sc).round() as i32;
    let (pad, width) = (px(PAD), px(WIDTH));
    let inner = width - 2 * pad;
    let mut y = pad;

    // Cabeçalho: FPS grande + tempo de quadro.
    let fps_col = match m.fps {
        Some(f) if f >= 60 => GOOD,
        Some(f) if f >= 30 => WARN,
        Some(_) => BAD,
        None => MUTED,
    };
    let base = y + px(40.0);
    let fps_txt = m.fps.map_or("--".to_string(), |f| f.to_string());
    let wv = p.text(pad, base, &fps_txt, FontKind::Hero, if m.fps.is_some() { TEXT } else { MUTED });
    p.text(pad + wv + px(6.0), base, "FPS", FontKind::Unit, fps_col);
    if let Some(f) = m.fps.filter(|&f| f > 0) {
        let wu = p.text_right(width - pad, base, "ms", FontKind::Unit, MUTED);
        p.text_right(width - pad - wu - px(4.0), base, &format!("{:.1}", 1000.0 / f as f32), FontKind::Value, TEXT);
    }
    if m.show_graph {
        y += px(54.0);
        p.sparkline(pad, y, inner, px(34.0), &m.hist, fps_col);
        y += px(34.0) + px(16.0);
    } else {
        y += px(50.0);
    }

    for sec in &m.sections {
        p.divider(pad, y, inner);
        y += px(16.0);

        p.circle(pad + px(4.0), y + px(10.0), px(4.0), sec.accent, 1.0);
        let tb = y + px(15.0);
        p.text(pad + px(16.0), tb, &sec.title, FontKind::Title, TEXT);
        if let Some(u) = sec.usage {
            let wu = p.text_right(width - pad, tb, "%", FontKind::Unit, MUTED);
            p.text_right(width - pad - wu - px(3.0), tb, &u.to_string(), FontKind::Title, TEXT);
        }
        y += px(28.0);

        if let Some(u) = sec.usage {
            let bh = px(6.0);
            p.rrect(pad, y, inner, bh, bh / 2, (255, 255, 255), 0.09);
            let fw = (inner as f32 * u.min(100) as f32 / 100.0).round() as i32;
            if fw > 0 {
                p.rrect(pad, y, fw.max(bh), bh, bh / 2, sec.accent, 1.0);
            }
            y += bh + px(16.0);
        }

        let colw = inner / 2;
        for (j, c) in sec.cells.iter().enumerate() {
            let cx = pad + (j as i32 % 2) * colw;
            let cy = y + (j as i32 / 2) * px(42.0);
            if let Some(kind) = c.icon {
                let s = px(16.0) as f32;
                p.icon(kind, cx as f32 + icon_width(kind, s) / 2.0, cy as f32 + px(5.0) as f32, s, ICON_COLOR, false);
            } else {
                p.text(cx, cy + px(10.0), &c.label, FontKind::Label, MUTED);
            }
            let vw = p.text(cx, cy + px(32.0), &c.value, FontKind::Value, c.color);
            p.text(cx + vw + px(3.0), cy + px(32.0), &c.unit, FontKind::Unit, MUTED);
        }
        y += (sec.cells.len() as i32 + 1) / 2 * px(42.0) + px(2.0);
    }
    y + pad - px(8.0)
}

// ------------------------------------------------------------------ montagem

fn create_dib(screen: HDC, w: i32, h: i32) -> Option<(HBITMAP, *mut u32)> {
    unsafe {
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
        let bmp = CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        Some((bmp, bits as *mut u32))
    }
}

/// Cria a fonte; se a "Segoe UI Variable" não existir (Windows anterior ao 11), cai na "Segoe UI".
fn make_font(dc: HDC, px: i32, weight: i32, display: bool) -> Font {
    unsafe {
        let faces = [
            if display { w!("Segoe UI Variable Display") } else { w!("Segoe UI Variable Text") },
            w!("Segoe UI"),
        ];
        let mut best = None;
        for face in faces {
            let h = CreateFontW(-px, 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY, 0, face);
            SelectObject(dc, h.into());
            let mut name = [0u16; 64];
            let n = GetTextFaceW(dc, Some(&mut name)) as usize;
            let got = String::from_utf16_lossy(&name[..n.saturating_sub(1).min(63)]);
            let want = face.to_string().unwrap_or_default();
            if got.eq_ignore_ascii_case(&want) || want == "Segoe UI" {
                best = Some(h);
                break;
            }
            let _ = DeleteObject(h.into());
        }
        let h = best.unwrap_or_else(|| CreateFontW(-px, 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY, 0, w!("Segoe UI")));
        SelectObject(dc, h.into());
        let mut tm = TEXTMETRICW::default();
        let _ = GetTextMetricsW(dc, &mut tm);
        Font { h, ascent: tm.tmAscent, height: tm.tmHeight }
    }
}

pub fn render(m: &Model, scale: f32, opacity_pct: u32) -> Option<Canvas> {
    let mut dry = Painter { dc: HDC::default(), out: null_mut(), cov: null_mut(), w: 0, h: 0, fonts: Vec::new(), scale, dry: true };
    let height = draw(&mut dry, m);
    let width = dry.px(WIDTH);

    unsafe {
        let screen = GetDC(None);
        let cov_dc = CreateCompatibleDC(Some(screen));
        let out_dc = CreateCompatibleDC(Some(screen));
        let (Some((cov_bmp, cov)), Some((out_bmp, out))) = (create_dib(screen, width, height), create_dib(screen, width, height)) else {
            let _ = DeleteDC(cov_dc);
            let _ = DeleteDC(out_dc);
            ReleaseDC(None, screen);
            return None;
        };
        let cov_old = SelectObject(cov_dc, cov_bmp.into());
        let out_old = SelectObject(out_dc, out_bmp.into());
        SetBkMode(cov_dc, TRANSPARENT);
        SetTextColor(cov_dc, COLORREF(0x00FF_FFFF));

        let px = |v: f32| (v * scale).round() as i32;
        let fonts = vec![
            make_font(cov_dc, px(46.0), 700, true),  // Hero
            make_font(cov_dc, px(15.0), 600, true),  // Title
            make_font(cov_dc, px(11.0), 500, false), // Label
            make_font(cov_dc, px(17.0), 600, true),  // Value
            make_font(cov_dc, px(12.0), 500, false), // Unit
        ];

        let mut p = Painter { dc: cov_dc, out, cov, w: width, h: height, fonts, scale, dry: false };
        p.card(opacity_pct);
        draw(&mut p, m);

        for f in &p.fonts {
            let _ = DeleteObject(f.h.into());
        }
        SelectObject(cov_dc, cov_old);
        let _ = DeleteObject(cov_bmp.into());
        let _ = DeleteDC(cov_dc);
        ReleaseDC(None, screen);
        Some(Canvas { dc: out_dc, bmp: out_bmp, old: out_old, w: width, h: height })
    }
}

// ------------------------------------------------------------------ barra de tarefas

/// Itens de uma linha da barra: texto simples ou um periférico (ícone + porcentagem).
pub enum BarItem {
    Text(String),
    Device { kind: Kind, percent: u32, charging: bool },
}

fn text_width(dc: HDC, s: &str) -> i32 {
    unsafe {
        let u: Vec<u16> = s.encode_utf16().collect();
        let mut sz = SIZE::default();
        let _ = GetTextExtentPoint32W(dc, &u, &mut sz);
        sz.cx
    }
}

/// Desenha as linhas da barra (texto + ícones de periféricos) sobre fundo transparente.
/// Bateria baixa (≤ 30 %) deixa o ícone e o número em âmbar; ≤ 15 %, em vermelho.
pub fn render_bar(lines: &[Vec<BarItem>], font_px: i32, light_text: bool) -> Option<Canvas> {
    unsafe {
        let screen = GetDC(None);
        let meas = CreateCompatibleDC(Some(screen));
        let font = make_font(meas, font_px, 600, false);
        let em = font_px as f32;
        let icon_s = em * 1.12;
        let gap_item = em.round() as i32;
        let gap_icon = (em * 0.3).round() as i32;
        let padx = (em * 0.5).round() as i32;
        let dev_text = |p: u32| format!("{p}%");

        let item_w = |it: &BarItem| -> i32 {
            match it {
                BarItem::Text(s) => text_width(meas, s),
                BarItem::Device { kind, percent, .. } => {
                    icon_width(*kind, icon_s).ceil() as i32 + gap_icon + text_width(meas, &dev_text(*percent))
                }
            }
        };
        let line_w: Vec<i32> = lines
            .iter()
            .map(|l| l.iter().map(item_w).sum::<i32>() + gap_item * (l.len() as i32 - 1).max(0))
            .collect();
        let width = line_w.iter().copied().max().unwrap_or(0) + 2 * padx;
        let height = font.height * lines.len() as i32;

        let release_meas = |meas: HDC| {
            SelectObject(meas, GetStockObject(SYSTEM_FONT));
            let _ = DeleteObject(font.h.into());
            let _ = DeleteDC(meas);
        };
        if width <= 2 * padx || height <= 0 {
            release_meas(meas);
            ReleaseDC(None, screen);
            return None;
        }

        let cov_dc = CreateCompatibleDC(Some(screen));
        let out_dc = CreateCompatibleDC(Some(screen));
        let (Some((cov_bmp, cov)), Some((out_bmp, out))) = (create_dib(screen, width, height), create_dib(screen, width, height)) else {
            let _ = DeleteDC(cov_dc);
            let _ = DeleteDC(out_dc);
            release_meas(meas);
            ReleaseDC(None, screen);
            return None;
        };
        let cov_old = SelectObject(cov_dc, cov_bmp.into());
        let out_old = SelectObject(out_dc, out_bmp.into());
        SetBkMode(cov_dc, TRANSPARENT);
        SetTextColor(cov_dc, COLORREF(0x00FF_FFFF));

        let mut p = Painter { dc: cov_dc, out, cov, w: width, h: height, fonts: vec![font; 5], scale: 1.0, dry: false };
        let text_col: Rgb = if light_text { (0xF2, 0xF2, 0xF2) } else { (0x20, 0x20, 0x20) };

        for (li, line) in lines.iter().enumerate() {
            let base = li as i32 * font.height + font.ascent;
            let mut x = padx;
            for it in line {
                match it {
                    BarItem::Text(s) => {
                        let w = p.text(x, base, s, FontKind::Value, text_col);
                        x += w + gap_item;
                    }
                    BarItem::Device { kind, percent, charging } => {
                        let col = if *percent <= 15 { BAD } else if *percent <= 30 { WARN } else { text_col };
                        let iw = icon_width(*kind, icon_s);
                        p.icon(*kind, x as f32 + iw / 2.0, base as f32 - em * 0.36, icon_s, col, *charging);
                        x += iw.ceil() as i32 + gap_icon;
                        let w = p.text(x, base, &dev_text(*percent), FontKind::Value, col);
                        x += w + gap_item;
                    }
                }
            }
        }

        SelectObject(cov_dc, cov_old);
        let _ = DeleteObject(cov_bmp.into());
        let _ = DeleteDC(cov_dc);
        release_meas(meas);
        ReleaseDC(None, screen);
        Some(Canvas { dc: out_dc, bmp: out_bmp, old: out_old, w: width, h: height })
    }
}