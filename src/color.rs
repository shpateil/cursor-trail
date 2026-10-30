// перцептивно ровная интерполяция цвета (OKLab) и генерация текстур-штампов
// в sRGB градиент розовый->фиолетовый даёт грязную середину, в OKLab — чистую

use tiny_skia::Pixmap;

pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

// sRGB -> OKLab, вход 0..1
pub fn to_oklab(r: f32, g: f32, b: f32) -> [f32; 3] {
    let (r, g, b) = (srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b));
    let l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
    let m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
    let s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;
    let (l_, m_, s_) = (l.cbrt(), m.cbrt(), s.cbrt());
    [
        0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
        1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
        0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
    ]
}

// OKLab -> sRGB, выход 0..1
pub fn from_oklab(lab: [f32; 3]) -> (f32, f32, f32) {
    let l_ = lab[0] + 0.3963377774 * lab[1] + 0.2158037573 * lab[2];
    let m_ = lab[0] - 0.1055613458 * lab[1] - 0.0638541728 * lab[2];
    let s_ = lab[0] - 0.0894841775 * lab[1] - 1.2914855480 * lab[2];
    let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    let r = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s;
    let g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s;
    let b = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s;
    (
        linear_to_srgb(r.clamp(0.0, 1.0)),
        linear_to_srgb(g.clamp(0.0, 1.0)),
        linear_to_srgb(b.clamp(0.0, 1.0)),
    )
}

// смешать два sRGB цвета в OKLab
pub fn mix(a: (f32, f32, f32), b: (f32, f32, f32), t: f32) -> (f32, f32, f32) {
    let t = t.clamp(0.0, 1.0);
    let la = to_oklab(a.0, a.1, a.2);
    let lb = to_oklab(b.0, b.1, b.2);
    let lab = [
        la[0] + (lb[0] - la[0]) * t,
        la[1] + (lb[1] - la[1]) * t,
        la[2] + (lb[2] - la[2]) * t,
    ];
    from_oklab(lab)
}

// ---- текстуры штампов ----
// генерируются один раз на старте, кладутся в память. внешних png не нужно.

#[derive(Clone, Copy, PartialEq)]
pub enum Shape {
    Blob,    // гауссово пятно, классический мягкий трейл
    Chevron, // стрелка как в osu
    Diamond, // ромб
    Star,    // четырёхлучевая звезда
    Ring,    // кольцо, «пузырь»
    Dot,     // жёсткая точка
}

impl Shape {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "blob" => Shape::Blob,
            "chevron" | "arrow" => Shape::Chevron,
            "diamond" => Shape::Diamond,
            "star" => Shape::Star,
            "ring" => Shape::Ring,
            "dot" => Shape::Dot,
            _ => return None,
        })
    }
}

// текстура 64x64, белая с альфой — цвет накладывается при рисовании
pub fn make_texture(shape: Shape) -> Pixmap {
    const S: u32 = 64;
    let mut pm = Pixmap::new(S, S).unwrap();
    let c = S as f32 / 2.0;
    let data = pm.data_mut();

    for y in 0..S {
        for x in 0..S {
            // координаты в -1..1
            let px = (x as f32 + 0.5 - c) / c;
            let py = (y as f32 + 0.5 - c) / c;
            let a = alpha_for(shape, px, py);
            if a <= 0.0 {
                continue;
            }
            let i = ((y * S + x) * 4) as usize;
            // premultiplied RGBA, цвет белый
            let v = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
            data[i] = v;
            data[i + 1] = v;
            data[i + 2] = v;
            data[i + 3] = v;
        }
    }
    pm
}

fn alpha_for(shape: Shape, x: f32, y: f32) -> f32 {
    let r = (x * x + y * y).sqrt();
    match shape {
        Shape::Blob => {
            // гауссово затухание, мягкий край
            if r >= 1.0 {
                return 0.0;
            }
            (-3.2 * r * r).exp() * (1.0 - r).max(0.0).powf(0.35)
        }
        Shape::Chevron => {
            // два пера: остриё в +X
            let tip = 0.82;
            let back = -0.62;
            let half = 0.58;
            // перо 1: от острия к (back, -half)
            let a1 = tri(x, y, tip, 0.0, back, -half, back * 0.42, 0.0);
            // перо 2: от острия к (back, +half)
            let a2 = tri(x, y, tip, 0.0, back, half, back * 0.42, 0.0);
            a1.max(a2)
        }
        Shape::Diamond => {
            let d = x.abs() + y.abs();
            if d >= 0.9 {
                return 0.0;
            }
            ((0.9 - d) * 3.4).min(1.0)
        }
        Shape::Star => {
            // 4 луча по осям
            let ang = y.atan2(x);
            let k = (ang * 2.0).cos().abs(); // 4 лепестка
            let reach = 0.28 + 0.72 * k.powf(3.0);
            if r >= reach {
                return 0.0;
            }
            ((reach - r) * 5.0).min(1.0)
        }
        Shape::Ring => {
            let w = 0.30;
            let d = (r - 0.62).abs();
            if d >= w {
                return 0.0;
            }
            (1.0 - d / w).powf(0.7)
        }
        Shape::Dot => {
            if r >= 0.55 {
                return 0.0;
            }
            ((0.55 - r) * 8.0).min(1.0)
        }
    }
}

// барицентрическая проверка точки в треугольнике, возвращает сглаженную альфу
fn tri(x: f32, y: f32, x1: f32, y1: f32, x2: f32, y2: f32, x3: f32, y3: f32) -> f32 {
    let d = (y2 - y3) * (x1 - x3) + (x3 - x2) * (y1 - y3);
    if d.abs() < 1e-6 {
        return 0.0;
    }
    let a = ((y2 - y3) * (x - x3) + (x3 - x2) * (y - y3)) / d;
    let b = ((y3 - y1) * (x - x3) + (x1 - x3) * (y - y3)) / d;
    let c = 1.0 - a - b;
    if a < 0.0 || b < 0.0 || c < 0.0 {
        return 0.0;
    }
    // мягкий край через минимальное расстояние до сторон
    let m = a.min(b).min(c);
    (m * 6.0).min(1.0)
}

// загрузка внешней текстуры (звезда osu), падает на blob если файла нет
pub fn load_texture_png(path: &str) -> Option<Pixmap> {
    let pm = Pixmap::load_png(path).ok()?;
    // буфер BGRA, png грузится как RGBA — меняем каналы
    let mut out = pm;
    for px in out.data_mut().chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    Some(out)
}
