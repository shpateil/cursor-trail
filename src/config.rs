// конфиг и темы для cursor-trail
// читается из ~/.config/cursor-trail/config.toml, переопределяется CLI-флагами

use std::{env, fs, path::PathBuf};

// brush = лента, stamps = штампы (форма задаётся в shape)
pub const STYLE_BRUSH: u8 = 0;
pub const STYLE_STAMPS: u8 = 1;
pub const STYLE_STARS: u8 = 2;

#[derive(Clone, Copy, Debug)]
pub struct Color3 {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Color3 {
    pub const fn new(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b }
    }
    pub fn from_hex(hex: &str) -> Option<Self> {
        let h = hex.trim().trim_start_matches('#');
        if h.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(h, 16).ok()?;
        Some(Self {
            r: ((v >> 16) & 0xff) as f32 / 255.0,
            g: ((v >> 8) & 0xff) as f32 / 255.0,
            b: (v & 0xff) as f32 / 255.0,
        })
    }
    pub fn rgb(self) -> (f32, f32, f32) {
        (self.r, self.g, self.b)
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub life: f32,      // мс жизни следа
    pub max_width: f32, // толщина у головы / размер штампа
    pub taper: f32,     // 1.0 = сужается в ноль, 0.0 = не сужается
    pub style: u8,      // STYLE_BRUSH / STYLE_STAMPS
    pub shape: String,  // форма штампа: blob chevron diamond star ring dot
    pub color_head: Color3,
    pub color_tail: Color3,
    pub glow: f32,              // сила ореола (первый проход), 0 = выкл
    pub thickness: f32,         // общий множитель толщины ленты
    pub smoothing: i32,         // проходы Chaikin: 0 = ломаная, 2 = гладко
    pub activation_speed: f32,  // px/сек: ниже этого след не растёт по ширине
    // параметры звёзд (стиль stars, физика как в osu StarFountain)
    pub star_speed: f32,        // начальная скорость частицы, px/сек
    pub star_spread: f32,       // разброс по перпендикуляру, px/сек
    pub star_gravity: f32,      // множитель гравитации 800
    pub star_life_mul: f32,     // множитель жизни 300..1000 мс
    pub spawn_every: f32,       // px пути между спавнами
    pub max_particles: usize,   // потолок частиц
    pub alpha_head: f32,        // непрозрачность у головы
    pub density: f32,           // множитель плотности штампов (1.0 = как в osu)
    pub fade_exp: f32,          // степень затухания (osu использует 1.7)
    pub spin: f32,              // доворот штампов по пути, 0 = без вращения
    pub rotate_to_motion: bool, // ориентировать штамп по направлению движения
}

impl Default for Config {
    fn default() -> Self {
        Self {
            life: 420.0,
            max_width: 9.0,
            taper: 0.85,
            style: STYLE_BRUSH,
            shape: "blob".into(),
            color_head: Color3::new(1.0, 0.373, 0.851), // #ff5fd9
            color_tail: Color3::new(0.545, 0.361, 0.965), // #8b5cf6
            glow: 0.55,
            thickness: 1.0,
            smoothing: 2,
            activation_speed: 320.0,
            star_speed: 260.0,
            star_spread: 70.0,
            star_gravity: 1.0,
            star_life_mul: 1.0,
            spawn_every: 14.0,
            max_particles: 400,
            alpha_head: 0.92,
            density: 1.0,
            fade_exp: 1.7,
            spin: 0.0,
            rotate_to_motion: true,
        }
    }
}

fn config_path() -> Option<PathBuf> {
    let base = env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|_| env::var("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok()?;
    Some(base.join("cursor-trail").join("config.toml"))
}

// простой парсер key = value, без секций
pub fn load_file() -> Config {
    let mut cfg = Config::default();
    let Some(p) = config_path() else { return cfg };
    let Ok(txt) = fs::read_to_string(&p) else {
        return cfg;
    };
    for line in txt.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim().trim_matches('"'));
        let v = v.split('#').next().unwrap_or(v).trim();
        match k {
            "life" => cfg.life = v.parse().unwrap_or(cfg.life),
            "max_width" => cfg.max_width = v.parse().unwrap_or(cfg.max_width),
            "taper" => cfg.taper = v.parse().unwrap_or(cfg.taper),
            "style" => {
                cfg.style = match v {
                    "brush" => STYLE_BRUSH,
                    "stamps" | "arrow" | "particle" => STYLE_STAMPS,
                    "stars" => STYLE_STARS,
                    _ => cfg.style,
                }
            }
            "shape" => cfg.shape = v.to_string(),
            "color_head" | "head" => {
                if let Some(c) = Color3::from_hex(v) {
                    cfg.color_head = c
                }
            }
            "color_tail" | "tail" => {
                if let Some(c) = Color3::from_hex(v) {
                    cfg.color_tail = c
                }
            }
            "glow" => cfg.glow = v.parse().unwrap_or(cfg.glow),
            "thickness" => cfg.thickness = v.parse().unwrap_or(cfg.thickness),
            "smoothing" => cfg.smoothing = v.parse().unwrap_or(cfg.smoothing),
            "activation_speed" => cfg.activation_speed = v.parse().unwrap_or(cfg.activation_speed),
            "star_speed" => cfg.star_speed = v.parse().unwrap_or(cfg.star_speed),
            "star_spread" => cfg.star_spread = v.parse().unwrap_or(cfg.star_spread),
            "star_gravity" => cfg.star_gravity = v.parse().unwrap_or(cfg.star_gravity),
            "star_life" => cfg.star_life_mul = v.parse().unwrap_or(cfg.star_life_mul),
            "spawn_every" => cfg.spawn_every = v.parse().unwrap_or(cfg.spawn_every),
            "max_particles" => cfg.max_particles = v.parse().unwrap_or(cfg.max_particles),
            "alpha_head" | "alpha" => cfg.alpha_head = v.parse().unwrap_or(cfg.alpha_head),
            "density" | "spacing" => cfg.density = v.parse().unwrap_or(cfg.density),
            "fade_exp" => cfg.fade_exp = v.parse().unwrap_or(cfg.fade_exp),
            "spin" => cfg.spin = v.parse().unwrap_or(cfg.spin),
            "rotate_to_motion" => cfg.rotate_to_motion = v == "true" || v == "1",
            _ => {}
        }
    }
    cfg
}

pub fn parse_args() -> Config {
    let mut cfg = load_file();
    let mut it = env::args().skip(1);
    while let Some(a) = it.next() {
        let mut num = |dst: &mut f32| match it.next().and_then(|v| v.parse().ok()) {
            Some(v) => *dst = v,
            None => {
                eprintln!("bad value for {a}");
                std::process::exit(2);
            }
        };
        match a.as_str() {
            "--life" => num(&mut cfg.life),
            "--max-width" => num(&mut cfg.max_width),
            "--taper" => num(&mut cfg.taper),
            "--glow" => num(&mut cfg.glow),
            "--thickness" => num(&mut cfg.thickness),
            "--smoothing" => { let mut v=cfg.smoothing as f32; num(&mut v); cfg.smoothing=v as i32 },
            "--activation" => num(&mut cfg.activation_speed),
            "--star-speed" => num(&mut cfg.star_speed),
            "--star-spread" => num(&mut cfg.star_spread),
            "--star-gravity" => num(&mut cfg.star_gravity),
            "--star-life" => num(&mut cfg.star_life_mul),
            "--spawn-every" => num(&mut cfg.spawn_every),
            "--max-particles" => { let mut v = cfg.max_particles as f32; num(&mut v); cfg.max_particles = v.max(10.0) as usize }
            "--alpha" => num(&mut cfg.alpha_head),
            "--density" | "--spacing" => num(&mut cfg.density),
            "--fade" => num(&mut cfg.fade_exp),
            "--spin" => num(&mut cfg.spin),
            "--no-rotate" => cfg.rotate_to_motion = false,
            "--rotate" => cfg.rotate_to_motion = true,
            "--shape" => {
                cfg.shape = it.next().unwrap_or_else(|| {
                    eprintln!("--shape needs a value");
                    std::process::exit(2)
                })
            }
            "--style" | "--theme" => {
                let v = it.next().unwrap_or_default();
                match v.as_str() {
                    "brush" => cfg.style = STYLE_BRUSH,
                    "stamps" | "arrow" | "particle" => cfg.style = STYLE_STAMPS,
                    "stars" => cfg.style = STYLE_STARS,
                    _ => {
                        eprintln!("style: brush | stamps");
                        std::process::exit(2)
                    }
                }
            }
            "--head" => {
                let v = it.next().unwrap_or_default();
                match Color3::from_hex(&v) {
                    Some(c) => cfg.color_head = c,
                    None => {
                        eprintln!("bad hex for --head: {v}");
                        std::process::exit(2)
                    }
                }
            }
            "--tail" => {
                let v = it.next().unwrap_or_default();
                match Color3::from_hex(&v) {
                    Some(c) => cfg.color_tail = c,
                    None => {
                        eprintln!("bad hex for --tail: {v}");
                        std::process::exit(2)
                    }
                }
            }
            "--dump-png" => {
                let _ = it.next();
            }
            "-h" | "--help" => {
                println!(
                    "cursor-trail [опции]\n\
                     \n\
                     форма:\n\
                     \x20 --style brush|stamps|stars   лента, штампы или звёзды (brush)\n\
                     \x20 --shape blob|chevron|diamond|star|ring|dot  форма штампа (blob)\n\
                     \x20 --rotate / --no-rotate   ориентировать штамп по движению (вкл)\n\
                     \n\
                     вид:\n\
                     \x20 --life <мс>        сколько живёт след (420)\n\
                     \x20 --max-width <px>   толщина/размер штампа (9)\n\
                     \x20 --taper <0..1>     сужение к хвосту (0.85)\n\
                     \x20 --head <hex>       цвет у курсора (#ff5fd9)\n\
                     \x20 --tail <hex>       цвет на хвосте (#8b5cf6)\n\
                     \x20 --glow <0..1>      ореол (0.55)
                     \x20 --thickness <мн>   общая толщина (1.0)
                     \x20 --smoothing <0..4> сглаживание Chaikin (2)
                     \x20 --activation <px/s> порог скорости (320)\n\
                     \x20 --alpha <0..1>     непрозрачность (0.92)\n\
                     \x20 --density <мн>     плотность штампов, 1.0 = как в osu\n\
                     \x20 --fade <мн>        степень затухания (1.7)\n\
                     \x20 --spin <0..1>      доворот штампов по пути (0)\n\
                     \n\
                     темы: cursor-trail-theme list"
                );
                std::process::exit(0);
            }
            _ => {
                eprintln!("unknown argument: {a}  (--help)");
                std::process::exit(2);
            }
        }
    }
    cfg.life = cfg.life.max(1.0);
    cfg.max_width = cfg.max_width.clamp(0.5, 120.0);
    cfg.taper = cfg.taper.clamp(0.0, 1.0);
    cfg.glow = cfg.glow.clamp(0.0, 1.0);
    cfg.alpha_head = cfg.alpha_head.clamp(0.05, 1.0);
    cfg.density = cfg.density.clamp(0.2, 5.0);
    cfg.fade_exp = cfg.fade_exp.clamp(0.3, 6.0);
    cfg.thickness = cfg.thickness.clamp(0.15, 5.0);
    cfg.smoothing = cfg.smoothing.clamp(0, 4);
    cfg
}
