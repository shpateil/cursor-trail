// рендер следа. модель взята из windtrail (wesleyyach, KWin-эффект, C++):
//  - сэмплы курсора со скоростью, сглаживание Chaikin (2 прохода)
//  - лента из квадов с РАЗНОЙ шириной на концах отрезка
//  - ширина зависит от скорости: медленно = тонко, быстро = толсто
//  - время жизни тоже зависит от скорости
//  - три прохода: ореол (2.05x, alpha 0.11), тело (1.0x, 0.38), ядро (0.27x, 0.78)
//  - квады раскладываются по 8 корзинам по "жизни", альфа = ((n+1)/8)^1.75
//
// это и даёт «красиво»: без сглаживания и без зависимости от скорости
// лента выглядит как мышиный хвост-палка, что бы ты ни крутил.

use std::time::Instant;
use tiny_skia::{Color, FillRule, PathBuilder, Pixmap, PixmapMut, PixmapPaint, Transform};

use crate::color::{make_texture, mix, Shape};
use crate::config::{Config, STYLE_BRUSH, STYLE_STARS};
use crate::particles::Fountain;

#[derive(Clone, Copy)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
    pub t: Instant,
}

#[derive(Clone, Copy)]
struct Sample {
    x: f32,
    y: f32,
    age_ms: f32,
    t_abs: std::time::Instant,
    speed: f32, // px/сек
}

pub struct Painter {
    tex: Pixmap,
    star: Option<Pixmap>,
    halo_tex: Pixmap,
    // состояние частиц живёт МЕЖДУ кадрами: если создавать фонтан заново
    // на каждом кадре, звёзды телепортируются и вместо следа получается каша
    fountain: std::cell::RefCell<Fountain>,
}

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

// как в windtrail: x*x*(3-2x)
fn smooth_step(v: f32) -> f32 {
    let x = clamp01(v);
    x * x * (3.0 - 2.0 * x)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

impl Painter {
    pub fn new(cfg: &Config) -> Self {
        let shape = Shape::parse(&cfg.shape).unwrap_or(Shape::Blob);
        // звезда: сначала ищем рядом с бинарём, потом в assets проекта
        let star = crate::color::load_texture_png(&star_path())
            .or_else(|| Some(make_texture(Shape::Star)));
        Self {
            tex: make_texture(shape),
            star,
            halo_tex: make_texture(Shape::Blob),
            fountain: std::cell::RefCell::new(Fountain::new()),
        }
    }

    pub fn paint(
        &self,
        pm: &mut PixmapMut,
        pts: &[Pt],
        (ox, oy): (i32, i32),
        s: f32,
        now: Instant,
        cfg: &Config,
    ) -> Option<(f32, f32, f32, f32)> {
        // при пустом следе звёзды всё равно должны дорисовываться и гаснуть,
        // иначе они застывают на экране навсегда
        if pts.len() < 2 {
            if cfg.style == STYLE_STARS {
                return self.paint_stars(pm, &[], cfg, s);
            }
            return None;
        }
        // мировые -> буферные, сразу считаем возраст и скорость
        let mut samples: Vec<Sample> = Vec::with_capacity(pts.len());
        for (i, p) in pts.iter().enumerate() {
            let x = (p.x - ox as f32) * s;
            let y = (p.y - oy as f32) * s;
            let age_ms = now.duration_since(p.t).as_secs_f32() * 1000.0;
            let speed = if i == 0 {
                0.0
            } else {
                let prev = &pts[i - 1];
                let dx = (p.x - prev.x) * s;
                let dy = (p.y - prev.y) * s;
                let dt = (p.t.duration_since(prev.t)).as_secs_f32().max(0.001);
                (dx * dx + dy * dy).sqrt() / dt
            };
            samples.push(Sample {
                x,
                y,
                age_ms,
                t_abs: p.t,
                speed,
            });
        }

        // скорость сглаживаем: одиночные скачки дают рваную ширину.
        // ema назад по хвосту: чем дальше сегмент от головы, тем сильнее сглажен,
        // у головы скорость живая (ширина реагирует мгновенно), в хвосте ровная
        for i in 1..samples.len() {
            // коэффициент тем меньше, чем старее точка: у головы ~0.5, в хвосте ~0.1
            let k = 0.5 / (1.0 + i as f32 * 0.35);
            samples[i].speed = lerp(samples[i].speed, samples[i - 1].speed, 1.0 - k);
        }

        // интерполяция головы: ipc приходит раз в ~16мс и на быстром взмахе даёт
        // прыжки по 30-80px, chaikin между двумя далёкими точками не спасает и
        // голова дёргается. досыпаем точки так, чтобы соседние были не дальше
        // 6px: плотность растёт только на длинных прыжках, на медленном ходе
        // ничего не меняется
        const MAX_SEG: f32 = 6.0;
        let mut dense: Vec<Sample> = Vec::with_capacity(samples.len() * 2);
        dense.push(samples[0]);
        for w in samples.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            let len = (dx * dx + dy * dy).sqrt();
            let steps = (len / MAX_SEG).ceil().max(1.0) as usize;
            for k in 1..=steps {
                let t = k as f32 / steps as f32;
                dense.push(Sample {
                    x: a.x + dx * t,
                    y: a.y + dy * t,
                    age_ms: lerp(a.age_ms, b.age_ms, t),
                    t_abs: if t < 1.0 { a.t_abs } else { b.t_abs },
                    speed: lerp(a.speed, b.speed, t),
                });
            }
        }
        let samples: Vec<Sample> = dense;

        if cfg.style == STYLE_BRUSH {
            // catmull-rom вместо chaikin: строит гладкую кривую ЧЕРЕЗ точки и
            // скругляет углы до дуг, chaikin на плотной сетке режет угол всего
            // на пару пикселей и углы остаются заметными
            let smoothed = if cfg.smoothing > 0 {
                catmull_rom(&samples, 4)
            } else {
                samples
            };
            self.paint_ribbon(pm, &smoothed, cfg, s)
        } else if cfg.style == STYLE_STARS {
            self.paint_stars(pm, &samples, cfg, s)
        } else {
            self.paint_stamps(pm, &samples, cfg, s);
            None
        }
    }


    // лента как в windtrail: три прохода, квады, корзины по жизни
    fn paint_ribbon(&self, pm: &mut PixmapMut, samples: &[Sample], cfg: &Config, s: f32) -> Option<(f32, f32, f32, f32)> {
        if samples.len() < 2 {
            return None;
        }

        // гасим весь след когда курсор встал (в windtrail это stopFade)
        let newest_age = samples.last().map(|p| p.age_ms).unwrap_or(0.0);
        let newest_life = lifetime_for(samples.last().map(|p| p.speed).unwrap_or(0.0), cfg);
        let fade_dur = (cfg.life * 0.44).clamp(65.0, 220.0);
        let fade_start = (newest_life - fade_dur).max(0.0);
        let stop_fade = if newest_age > fade_start {
            clamp01((newest_life - newest_age) / fade_dur)
        } else {
            1.0
        };
        if stop_fade <= 0.001 {
            return None;
        }

        // ореол, тело, ядро: (множитель ширины, альфа) — как в windtrail
        let passes: [(f32, f32); 3] = [
            (2.05, 0.11 * cfg.glow.max(0.05)),
            (1.00, 0.38),
            (0.27, 0.78),
        ];

        let full = cfg.activation_speed + 1530.0;

        // нормали в стыках: среднее нормалей смежных сегментов. это стандарт для
        // лент переменной ширины (vtk, шейдерные polyline-стрипы): соседние квады
        // используют ОДНИ и те же точки кромки на стыке, поэтому разрывов нет
        // ни снаружи поворота (дыры), ни внутри (наложения с двойной альфой).
        let n = samples.len();
        let mut jn_x = vec![0.0f32; n];
        let mut jn_y = vec![0.0f32; n];
        let seg_normal = |i0: usize, i1: usize| -> (f32, f32) {
            let dx = samples[i1].x - samples[i0].x;
            let dy = samples[i1].y - samples[i0].y;
            let len = (dx * dx + dy * dy).sqrt();
            if len < 0.01 {
                (0.0, 0.0)
            } else {
                (-dy / len, dx / len)
            }
        };
        for i in 0..n {
            let (ax, ay) = if i > 0 { seg_normal(i - 1, i) } else { (0.0, 0.0) };
            let (bx, by) = if i + 1 < n { seg_normal(i, i + 1) } else { (0.0, 0.0) };
            let (sx, sy) = (ax + bx, ay + by);
            let sl = (sx * sx + sy * sy).sqrt();
            // вырожденные случаи: нулевой сегмент или разворот на 180 градусов,
            // там усреднение даёт ноль — берём нормаль предыдущего сегмента
            let (ux, uy) = if sl > 0.05 {
                (sx / sl, sy / sl)
            } else if ax * ax + ay * ay > 0.0001 {
                (ax, ay)
            } else {
                (bx, by)
            };
            jn_x[i] = ux;
            jn_y[i] = uy;
        }

        // 16 корзин по жизни вместо 8: шаг альфы/цвета между соседними корзинами
        // вдвое меньше, переходы не видны как ступеньки
        const BUCKETS: usize = 16;

        // аддитивный ореол: свет складывается с фоном, это и даёт неоновое
        // свечение как в osu. тело и ядро остаются обычной краской, иначе на
        // светлом фоне след исчезает (аддитив к белому ничего не даёт видимого)
        for (pass_idx, &(width_scale, base_alpha)) in passes.iter().enumerate() {
            let darkening = if pass_idx == 0 { 0.28 } else { 0.0 };
            let whitening = if pass_idx == 2 { 0.72 } else { 0.0 };

            let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); BUCKETS];
            for i in 0..samples.len() - 1 {
                let a = &samples[i];
                let b = &samples[i + 1];
                let life_a = clamp01(1.0 - a.age_ms / lifetime_for(a.speed, cfg));
                let life_b = clamp01(1.0 - b.age_ms / lifetime_for(b.speed, cfg));
                let seg_life = (tail_curve(life_a) + tail_curve(life_b)) * 0.5;
                if seg_life <= 0.002 {
                    continue;
                }
                let bucket = ((seg_life * BUCKETS as f32).floor() as usize).min(BUCKETS - 1);
                buckets[bucket].push(i);
            }

            for (bucket, segs) in buckets.iter().enumerate() {
                if segs.is_empty() {
                    continue;
                }
                let bucket_opacity = (((bucket + 1) as f32) / BUCKETS as f32).powf(1.75);
                let alpha = clamp01(base_alpha * bucket_opacity * stop_fade);
                if alpha <= 0.004 {
                    continue;
                }

                let t_col = (bucket as f32 + 0.5) / BUCKETS as f32;
                let (mut r, mut g, mut bb) =
                    mix(cfg.color_tail.rgb(), cfg.color_head.rgb(), t_col.powf(0.8));
                if darkening > 0.0 {
                    r *= 1.0 - darkening * 0.6;
                    g *= 1.0 - darkening * 0.6;
                    bb *= 1.0 - darkening * 0.6;
                }
                if whitening > 0.0 {
                    r = lerp(r, 1.0, whitening * 0.55);
                    g = lerp(g, 1.0, whitening * 0.55);
                    bb = lerp(bb, 1.0, whitening * 0.55);
                }

                // все квады корзины в один путь: наложение не удваивает альфу
                let mut pb = PathBuilder::new();
                let mut any = false;
                // стежки на головных рёбрах: стык с соседней (более старой) корзиной,
                // там антиалиасинг двух разных fill'ов даёт волосяную щель
                let mut stitch_at: Option<(f32, f32, f32, f32, f32)> = None;
                for &i in segs {
                    let (a, b) = (&samples[i], &samples[i + 1]);
                    let life_a = clamp01(1.0 - a.age_ms / lifetime_for(a.speed, cfg));
                    let life_b = clamp01(1.0 - b.age_ms / lifetime_for(b.speed, cfg));
                    let sp_a = smooth_step((a.speed - cfg.activation_speed) / (full - cfg.activation_speed).max(1.0));
                    let sp_b = smooth_step((b.speed - cfg.activation_speed) / (full - cfg.activation_speed).max(1.0));
                    let wa = width_for(life_a, sp_a, cfg) * width_scale;
                    let wb = width_for(life_b, sp_b, cfg) * width_scale;
                    // хвост сводится в точку на последних 6% жизни: плоский срез
                    // на конце выглядит обрубленным, нужен мягкий кончик
                    let tip_a = smooth_step(life_a / 0.06);
                    let tip_b = smooth_step(life_b / 0.06);
                    let wa = wa * tip_a;
                    let wb = wb * tip_b;
                    if push_quad(
                        &mut pb,
                        a.x, a.y, jn_x[i], jn_y[i], wa,
                        b.x, b.y, jn_x[i + 1], jn_y[i + 1], wb,
                    ) {
                        any = true;
                    }
                    // головное ребро корзины: соседний сегмент уже в другой корзине
                    // (иначе бы попал в segs). стежок только если справа есть сегмент
                    if i + 2 < samples.len() && seg_life_of(i + 1, samples, cfg) < bucket_start(bucket + 1) {
                        stitch_at = Some((b.x, b.y, jn_x[i + 1], jn_y[i + 1], wb * 0.5));
                    }
                    // голова следа: круглый наконечник вместо плоского среза
                    if i + 2 == samples.len() {
                        pb.push_circle(b.x, b.y, wb * 0.5);
                        any = true;
                    }
                }
                if let Some((sx, sy, snx, sny, shw)) = stitch_at {
                    push_stitch(&mut pb, sx, sy, snx, sny, shw);
                    any = true;
                }
                if !any {
                    continue;
                }
                if let Some(path) = pb.finish() {
                    let mut paint = tiny_skia::Paint::default();
                    paint.set_color(to_color(r, g, bb, alpha));
                    paint.anti_alias = true;
                    // ореол складывается со свечением других корзин, свет копится
                    paint.blend_mode = if pass_idx == 0 {
                        tiny_skia::BlendMode::Plus
                    } else {
                        tiny_skia::BlendMode::SourceOver
                    };
                    pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
                }
            }
        }
        None
    }

    // остались ли живые звёзды: нужно главному циклу, чтобы дорисовывать
    // умирающие частицы уже после того как след опустел
    pub fn has_live_stars(&self) -> bool {
        let f = self.fountain.borrow();
        let now = std::time::Instant::now();
        f.stars
            .iter()
            .any(|st| now.duration_since(st.born).as_secs_f32() * 1000.0 <= st.dur)
    }

    // звёзды: фонтан частиц вдоль пути, физика из osu StarFountain.
    // возвращает bbox всех нарисованных звёзд: они улетают далеко от пути,
    // и без этого dirty rect обрезает их.
    fn paint_stars(&self, pm: &mut PixmapMut, samples: &[Sample], cfg: &Config, s: f32) -> Option<(f32, f32, f32, f32)> {
        let Some(tex) = self.star.as_ref() else { return None };
        let mut f = self.fountain.borrow_mut();

        if samples.is_empty() {
            // следа уже нет — только дорисовываем умирающие звёзды
            let now = std::time::Instant::now();
            f.prune(now);
            return self.draw_star_set(pm, &f, now, cfg, s, tex);
        }

        // samples идут от старой точки к новой. кормим только те, что реально
        // появились с прошлого кадра, иначе за 60 кадров в секунду спавним весь
        // след заново и звёзды телепортируются
        let now_instant = samples.last().map(|p| p.t_abs).unwrap_or_else(std::time::Instant::now);
        for smp in samples.iter() {
            if f.last_fed.map_or(true, |t| smp.t_abs > t) {
                f.feed(smp.x, smp.y, smp.t_abs, cfg);
            }
        }
        f.last_fed = Some(now_instant);
        f.prune(now_instant);

        let now_instant = samples.last().map(|p| p.t_abs).unwrap_or_else(std::time::Instant::now);
        self.draw_star_set(pm, &f, now_instant, cfg, s, tex)
    }

    // отрисовка текущего набора звёзд: общая для «есть след» и «след кончился»
    fn draw_star_set(
        &self,
        pm: &mut PixmapMut,
        f: &Fountain,
        now_instant: std::time::Instant,
        cfg: &Config,
        s: f32,
        tex: &Pixmap,
    ) -> Option<(f32, f32, f32, f32)> {
        let pp = PixmapPaint {
            opacity: 1.0,
            blend_mode: tiny_skia::BlendMode::SourceOver,
            quality: tiny_skia::FilterQuality::Bilinear,
        };
        let tw = tex.width() as f32;
        let mut bbox: Option<(f32, f32, f32, f32)> = None;

        for st in &f.stars {
            let Some(state) = f.state(st, now_instant, cfg.star_gravity) else {
                continue;
            };
            // звёзды рождаются от старых точек: их возраст меньше, значит t ближе к 0
            let life = 1.0 - state.t;
            let (r, g, b) = mix(cfg.color_tail.rgb(), cfg.color_head.rgb(), life);
            let alpha = cfg.alpha_head * life.powf(0.9);
            if alpha <= 0.01 {
                continue;
            }
            // max_width это ДИАМЕТР звезды в пикселях, а не множитель к текстуре:
            // иначе при текстуре 66px звёзды выходили по 66-145px и выглядели кашей
            let sz = cfg.max_width * state.scale * s;
            let half = sz / 2.0;

            // мягкий ореол под звездой: даёт объём и «свет», без него точки плоские
            if cfg.glow > 0.01 {
                let halo = sz * 2.6;
                let hhalf = halo / 2.0;
                let htint = self.tinted(&self.halo_tex, r, g, b, alpha * cfg.glow * 0.30);
                let htr = Transform::from_scale(halo / self.halo_tex.width() as f32,
                                                halo / self.halo_tex.height() as f32)
                    .post_concat(Transform::from_translate(state.x - hhalf, state.y - hhalf));
                pm.draw_pixmap(0, 0, htint.as_ref(), &pp, htr, None);
            }

            let tinted = self.tinted(tex, r, g, b, alpha);
            let tr = Transform::from_scale(sz / tw, sz / tw)
                .post_concat(Transform::from_rotate(state.angle))
                .post_concat(Transform::from_translate(state.x - half, state.y - half));
            pm.draw_pixmap(0, 0, tinted.as_ref(), &pp, tr, None);
            // учитываем габарит в bbox
            let (px0, py0) = (state.x - half, state.y - half);
            let (px1, py1) = (px0 + sz, py0 + sz);
            bbox = Some(match bbox {
                None => (px0, py0, px1, py1),
                Some(b) => (b.0.min(px0), b.1.min(py0), b.2.max(px1), b.3.max(py1)),
            });
        }
        bbox
    }

    // штампы: форма из color.rs, интервал как в osu (размер / 2.5)
    fn paint_stamps(&self, pm: &mut PixmapMut, samples: &[Sample], cfg: &Config, s: f32) -> Option<(f32, f32, f32, f32)> {
        let size = cfg.max_width * s;
        let interval = (size / 2.5 * cfg.density).max(1.0);
        let pp = PixmapPaint {
            opacity: 1.0,
            blend_mode: tiny_skia::BlendMode::SourceOver,
            quality: tiny_skia::FilterQuality::Bilinear,
        };

        let mut stamps: Vec<(f32, f32, f32, f32)> = Vec::new();
        for i in 0..samples.len() - 1 {
            let (a, b) = (samples[i], samples[i + 1]);
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            let len = (dx * dx + dy * dy).sqrt();
            if len < 0.001 {
                continue;
            }
            let ang = dy.atan2(dx);
            let n = (len / interval).ceil() as usize;
            for k in 0..n {
                let t = k as f32 / n.max(1) as f32;
                stamps.push((a.x + dx * t, a.y + dy * t, ang, lerp(a.age_ms, b.age_ms, t)));
            }
        }

        for (x, y, ang, age) in stamps {
            let life = clamp01(1.0 - age / cfg.life);
            if life <= 0.0 {
                continue;
            }
            let (r, g, b) = mix(cfg.color_tail.rgb(), cfg.color_head.rgb(), life);
            let a = cfg.alpha_head * tail_curve(life);
            let sz = size * (1.0 - cfg.taper * (1.0 - life)).max(0.12);
            let tinted = self.tinted(&self.tex, r, g, b, a);
            let angle_deg = if cfg.spin > 0.001 {
                ang.to_degrees() + cfg.spin * 360.0 * life
            } else if cfg.rotate_to_motion {
                ang.to_degrees()
            } else {
                0.0
            };
            let half = sz / 2.0;
            let tr = Transform::from_scale(sz / 64.0, sz / 64.0)
                .post_concat(Transform::from_rotate(angle_deg))
                .post_concat(Transform::from_translate(x - half, y - half));
            pm.draw_pixmap(0, 0, tinted.as_ref(), &pp, tr, None);
        }
        None
    }

    fn tinted(&self, src: &Pixmap, r: f32, g: f32, b: f32, a: f32) -> Pixmap {
        let mut out = src.clone();
        let d = out.data_mut();
        let (r, g, b, a) = (clamp01(r), clamp01(g), clamp01(b), clamp01(a));
        for px in d.chunks_exact_mut(4) {
            let na = (px[3] as f32 / 255.0) * a;
            px[0] = (b * na * 255.0).round() as u8;
            px[1] = (g * na * 255.0).round() as u8;
            px[2] = (r * na * 255.0).round() as u8;
            px[3] = (na * 255.0).round() as u8;
        }
        out
    }
}

// ---- математика как в windtrail ----

// кривая хвоста: smoothstep в степени 1.55
fn tail_curve(life: f32) -> f32 {
    smooth_step(life).powf(1.55)
}

// время жизни зависит от скорости: медленно живёт меньше
fn lifetime_for(speed: f32, cfg: &Config) -> f32 {
    let full = cfg.activation_speed + 1530.0;
    let norm = smooth_step((speed - cfg.activation_speed) / (full - cfg.activation_speed).max(1.0));
    lerp((cfg.life * 0.53).max(90.0), cfg.life, norm)
}

// ширина как в windtrail: 0.18 + 17.5 * sqrt(speed_norm) * tail
// медленное движение = тонкая линия, быстрый взмах = толстая лента
fn width_for(life: f32, speed_norm: f32, cfg: &Config) -> f32 {
    (0.18 + 17.5 * speed_norm.sqrt() * tail_curve(life)) * cfg.thickness
}

// жизнь сегмента i (среднее по концам), для определения границ корзин
fn seg_life_of(i: usize, samples: &[Sample], cfg: &Config) -> f32 {
    let a = &samples[i];
    let b = &samples[i + 1];
    let life_a = clamp01(1.0 - a.age_ms / lifetime_for(a.speed, cfg));
    let life_b = clamp01(1.0 - b.age_ms / lifetime_for(b.speed, cfg));
    (tail_curve(life_a) + tail_curve(life_b)) * 0.5
}

// нижняя граница корзины (значение seg_life, с которого корзина начинается)
fn bucket_start(bucket: usize) -> f32 {
    bucket as f32 / 16.0
}

// добавляет квад между двумя точками. ширина задаётся в СТЫКАХ: кромка каждого
// стыка считается по усреднённой нормали, общей для обоих смежных квадов, поэтому
// на поворотах кромки соседей совпадают и щелей/наложений не возникает
fn push_quad(
    pb: &mut PathBuilder,
    x1: f32, y1: f32, nx1: f32, ny1: f32, w1: f32,
    x2: f32, y2: f32, nx2: f32, ny2: f32, w2: f32,
) -> bool {
    let (h1, h2) = (w1 * 0.5, w2 * 0.5);
    pb.move_to(x1 + nx1 * h1, y1 + ny1 * h1);
    pb.line_to(x2 + nx2 * h2, y2 + ny2 * h2);
    pb.line_to(x2 - nx2 * h2, y2 - ny2 * h2);
    pb.line_to(x1 - nx1 * h1, y1 - ny1 * h1);
    pb.close();
    true
}

// стежок на стыке двух корзин: соседние корзины это разные fill'ы и антиалиасинг
// на их общем ребре даёт волосяную щель. тонкая 1px накладка поперёк ленты ровно
// на ребре добивает покрытие только там где щель. перекрытие ВСЕЙ ленты не годится:
// на полупрозрачных заливках оно даёт полосы двойной плотности (проверено на практике)
fn push_stitch(pb: &mut PathBuilder, x: f32, y: f32, nx: f32, ny: f32, half_w: f32) {
    // касательная к ленте в стыке
    let (tx, ty) = (-ny, nx);
    let h = half_w + 0.5; // чуть шире ленты, накрывает aa кромок
    let t = 0.5; // полутолщина в обе стороны = 1px вдоль хода
    pb.move_to(x + nx * h + tx * t, y + ny * h + ty * t);
    pb.line_to(x + nx * h - tx * t, y + ny * h - ty * t);
    pb.line_to(x - nx * h - tx * t, y - ny * h - ty * t);
    pb.line_to(x - nx * h + tx * t, y - ny * h + ty * t);
    pb.close();
}

// catmull-rom сплайн: гладкая кривая через контрольные точки. samples_per_seg
// точек на сегмент, дублируем крайние точки чтобы кривая не обрезалась
fn catmull_rom(input: &[Sample], samples_per_seg: u32) -> Vec<Sample> {
    if input.len() < 3 {
        return input.to_vec();
    }
    let mut out: Vec<Sample> = Vec::with_capacity(input.len() * samples_per_seg as usize);
    let first = input[0];
    let last = *input.last().unwrap();
    let mut knots: Vec<Sample> = Vec::with_capacity(input.len() + 2);
    knots.push(Sample {
        x: 2.0 * first.x - input[1].x,
        y: 2.0 * first.y - input[1].y,
        age_ms: first.age_ms,
        t_abs: first.t_abs,
        speed: first.speed,
    });
    knots.extend_from_slice(input);
    knots.push(Sample {
        x: 2.0 * last.x - input[input.len() - 2].x,
        y: 2.0 * last.y - input[input.len() - 2].y,
        age_ms: last.age_ms,
        t_abs: last.t_abs,
        speed: last.speed,
    });
    let n_sub = samples_per_seg as usize;
    for i in 1..knots.len() - 2 {
        let (p0, p1, p2, p3) = (&knots[i - 1], &knots[i], &knots[i + 1], &knots[i + 2]);
        out.push(p1.clone());
        for k in 1..n_sub {
            let t = k as f32 / n_sub as f32;
            let t2 = t * t;
            let t3 = t2 * t;
            out.push(Sample {
                x: 0.5 * ((2.0 * p1.x) + (-p0.x + p2.x) * t
                    + (2.0 * p0.x - 5.0 * p1.x + 4.0 * p2.x - p3.x) * t2
                    + (-p0.x + 3.0 * p1.x - 3.0 * p2.x + p3.x) * t3),
                y: 0.5 * ((2.0 * p1.y) + (-p0.y + p2.y) * t
                    + (2.0 * p0.y - 5.0 * p1.y + 4.0 * p2.y - p3.y) * t2
                    + (-p0.y + 3.0 * p1.y - 3.0 * p2.y + p3.y) * t3),
                age_ms: lerp(p1.age_ms, p2.age_ms, t),
                t_abs: p1.t_abs,
                speed: lerp(p1.speed, p2.speed, t),
            });
        }
    }
    out.push(last);
    out
}

// сглаживание Chaikin: 2 прохода дают заметно глаже ломаную
fn chaikin(input: &[Sample], passes: usize) -> Vec<Sample> {
    let mut cur = input.to_vec();
    if passes == 0 || cur.len() < 3 {
        return cur;
    }
    for _ in 0..passes {
        let mut out: Vec<Sample> = Vec::with_capacity(cur.len() * 2);
        out.push(cur[0]);
        for w in cur.windows(2) {
            let (a, b) = (w[0], w[1]);
            out.push(Sample {
                x: 0.75 * a.x + 0.25 * b.x,
                y: 0.75 * a.y + 0.25 * b.y,
                age_ms: lerp(a.age_ms, b.age_ms, 0.25),
                t_abs: a.t_abs,
                speed: lerp(a.speed, b.speed, 0.25),
            });
            out.push(Sample {
                x: 0.25 * a.x + 0.75 * b.x,
                y: 0.25 * a.y + 0.75 * b.y,
                age_ms: lerp(a.age_ms, b.age_ms, 0.75),
                t_abs: b.t_abs,
                speed: lerp(a.speed, b.speed, 0.75),
            });
        }
        out.push(*cur.last().unwrap());
        cur = out;
    }
    cur
}

fn to_color(r: f32, g: f32, b: f32, a: f32) -> Color {
    // буфер Argb8888 (BGRA), Pixmap считает RGBA
    Color::from_rgba(b, g, r, clamp01(a)).unwrap_or(Color::BLACK)
}

// отладочный рендер: пишет картинку без экрана
pub fn dump_png(path: &str, cfg: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let mut pixmap = Pixmap::new(720, 480).ok_or("pixmap")?;
    pixmap.fill(Color::from_rgba8(0, 0, 0, 255));
    let now = Instant::now();
    let painter = Painter::new(cfg);

    // траектория с разной скоростью: медленный вход, разгон, торможение.
    // по ней видно зависимость ширины от скорости, как в windtrail.
    let mut pts = Vec::new();
    let steps = 120;
    for i in 0..steps {
        let k = i as f32 / (steps - 1) as f32;
        let ease = smooth_step(k);
        let age = (1.0 - ease) * cfg.life * 0.98;
        let t = now
            .checked_sub(std::time::Duration::from_millis(age as u64))
            .unwrap_or(now);
        let x = 70.0 + ease * 580.0;
        let y = 240.0 + (ease * 4.5).sin() * 95.0;
        pts.push(Pt { x, y, t });
    }
    painter.paint(&mut pixmap.as_mut(), &pts, (0, 0), 1.0, now, cfg);
    // второй кадр через 16 мс: частицы должны продвинуться, а не пересоздаться
    let now2 = now + std::time::Duration::from_millis(16);
    let pts2: Vec<Pt> = pts
        .iter()
        .map(|p| Pt { x: p.x + 4.0, y: p.y, t: p.t })
        .collect();
    painter.paint(&mut pixmap.as_mut(), &pts2, (0, 0), 1.0, now2, cfg);
    // сохраняем честно: BGRA -> RGBA
    for px in pixmap.data_mut().chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    pixmap.save_png(path)?;
    Ok(())
}

// где лежит текстура звезды: рядом с бинарём или в исходниках проекта
fn star_path() -> String {
    let candidates = [
        std::env::var("HOME").unwrap_or_default() + "/.local/share/cursor-trail/star.png",
        std::env::var("HOME").unwrap_or_default() + "/projects/cursor-trail/assets/star.png",
    ];
    for c in candidates {
        if std::path::Path::new(&c).exists() {
            return c;
        }
    }
    String::new()
}
