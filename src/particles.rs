// частицы-звёзды. физика взята из osu (StarFountain.cs):
//   ParticleGravity = 800
//   duration 300..1000 мс
//   StartAngle random +-4°, EndAngle random +-2°
//   EndScale = 2.2 +- 0.4
//   Velocity = (spread_x, -1400 +- 100)
//
// отличие от osu: звёзды рождаются вдоль движения курсора и летят в сторону
// движения, а не фонтаном вверх из одной точки. так это читается как след.
//
// время здесь в миллисекундах возраста: age_ms растёт от 0. так проще
// интерполировать между сэмплами траектории, у которых уже есть свой возраст.

const GRAVITY: f32 = 800.0;

#[derive(Clone, Copy)]
pub struct Star {
    pub x0: f32,
    pub y0: f32,
    pub vx: f32,
    pub vy: f32,
    pub born: std::time::Instant, // абсолютное время рождения
    pub dur: f32,
    pub spin: f32,
}

pub struct Fountain {
    pub stars: Vec<Star>,
    seed: u64,
    travel: f32,
    /// время самой свежей скормленной точки: по нему отличаем новые
    pub last_fed: Option<std::time::Instant>,
    /// последняя скормленная точка: нужна для направления движения
    last_point: Option<(f32, f32)>,
}

impl Fountain {
    pub fn new() -> Self {
        Self {
            stars: Vec::with_capacity(512),
            seed: 0x9E37_79B9_7F4A_7C15,
            travel: 0.0,
            last_fed: None,
            last_point: None,
        }
    }

    fn rnd(&mut self) -> f32 {
        // xorshift64*: детерминированно, без крэйта rand
        self.seed ^= self.seed >> 12;
        self.seed ^= self.seed << 25;
        self.seed ^= self.seed >> 27;
        let v = self.seed.wrapping_mul(0x2545_F491_4F6C_DD1D);
        ((v >> 40) as f32) / ((1u32 << 24) as f32)
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.rnd()
    }

    // кормим точками траектории: у каждой свой возраст в мс
    pub fn feed(&mut self, x: f32, y: f32, now: std::time::Instant, cfg: &crate::config::Config) {
        let (dx, dy, dist) = match self.last_point {
            Some((px, py)) => {
                let dx = x - px;
                let dy = y - py;
                (dx, dy, (dx * dx + dy * dy).sqrt())
            }
            None => (0.0, 0.0, 0.0),
        };
        self.last_point = Some((x, y));
        self.travel += dist;

        let every = cfg.spawn_every.max(2.0);
        while self.travel >= every {
            self.travel -= every;
            if self.stars.len() >= cfg.max_particles {
                break;
            }
            self.spawn(x, y, now, dx, dy, dist, cfg);
        }
    }

    fn spawn(&mut self, x: f32, y: f32, now: std::time::Instant, dx: f32, dy: f32, dist: f32, cfg: &crate::config::Config) {
        // направление движения, если стоим — вверх как в osu
        let (mut ux, mut uy) = if dist > 0.001 {
            (dx / dist, dy / dist)
        } else {
            (0.0, -1.0)
        };
        let speed = cfg.star_speed * self.range(0.85, 1.15);
        let spread = self.range(-cfg.star_spread, cfg.star_spread);
        // перпендикуляр
        let (nx, ny) = (-uy, ux);
        ux = ux * speed + nx * spread;
        uy = uy * speed + ny * spread;

        let dur = self.range(300.0, 1000.0) * cfg.star_life_mul;
        let spin = self.range(-40.0, 40.0);
        self.stars.push(Star {
            x0: x,
            y0: y,
            vx: ux,
            vy: uy,
            born: now,
            dur,
            spin,
        });
    }

    // убрать звёзды, которые уже отжили
    pub fn prune(&mut self, now: std::time::Instant) {
        self.stars
            .retain(|s| now.duration_since(s.born).as_secs_f32() * 1000.0 <= s.dur);
    }

    // состояние звезды на момент с текущим возрастом траектории now_age
    pub fn state(&self, s: &Star, now: std::time::Instant, gravity_mul: f32) -> Option<StarState> {
        let age = now.duration_since(s.born).as_secs_f32() * 1000.0;
        if age > s.dur {
            return None;
        }
        let t = age / s.dur;
        let sec = age / 1000.0;
        Some(StarState {
            x: s.x0 + s.vx * sec,
            y: s.y0 + s.vy * sec + 0.5 * GRAVITY * gravity_mul * sec * sec,
            t,
            angle: s.spin * t,
            // как в osu: масштаб растёт к EndScale +- 0.4
            scale: 0.55 + (2.2 - 0.55) * t,
        })
    }
}

pub struct StarState {
    pub x: f32,
    pub y: f32,
    pub t: f32,
    pub angle: f32,
    pub scale: f32,
}
