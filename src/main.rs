use std::{
    env,
    io::{Read, Write},
    os::unix::net::UnixStream,
    thread::sleep,
    time::{Duration, Instant},
};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_dispatch2, delegate_registry,
    output::{OutputHandler, OutputState},
    reexports::client::{
        globals::registry_queue_init,
        protocol::{wl_output, wl_shm, wl_surface},
        Connection, QueueHandle,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{
        slot::{Buffer, SlotPool},
        Shm, ShmHandler,
    },
};
use tiny_skia::PixmapMut;

mod color;
mod particles;
mod config;
mod paint;

use config::Config;
use paint::Painter;

const FAST: Duration = Duration::from_millis(16);
const IDLE: Duration = Duration::from_millis(50);

/// Rectangle in buffer pixels: [x0, y0, x1, y1).
#[derive(Clone, Copy, PartialEq)]
struct IRect {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl IRect {
    fn union(self, o: IRect) -> IRect {
        IRect {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }
}

fn union_opt(a: Option<IRect>, b: Option<IRect>) -> Option<IRect> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.union(b)),
        (a, None) => a,
        (None, b) => b,
    }
}

struct Out {
    output: wl_output::WlOutput,
    layer: LayerSurface,
    pool: Option<SlotPool>,
    buffers: Vec<Buffer>,
    /// What was last drawn into each buffer (so it can be cleared).
    dirty: [Option<IRect>; 2],
    last: Option<IRect>,
    idx: usize,
    w: i32, // rozmiar logiczny
    h: i32,
    scale: i32,
    configured: bool,
    released: bool,
}

struct App {
    registry: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    shm: Shm,
    layer_shell: LayerShell,
    outs: Vec<Out>,
    exit: bool,
}

impl App {
    fn add_output(&mut self, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some("cursor-trail"),
            Some(&output),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(0, 0);
        // Empty input region: clicks pass through the overlay.
        if let Ok(region) = smithay_client_toolkit::compositor::Region::new(&self.compositor) {
            layer.set_input_region(Some(region.wl_region()));
        }
        layer.commit();
        self.outs.push(Out {
            output,
            layer,
            pool: None,
            buffers: Vec::new(),
            dirty: [None; 2],
            last: None,
            idx: 0,
            w: 0,
            h: 0,
            scale: 1,
            configured: false,
            released: true,
        });
    }

    fn output_geometry(&self, o: &wl_output::WlOutput) -> (i32, i32, i32) {
        match self.output_state.info(o) {
            Some(i) => {
                let (x, y) = i.logical_position.unwrap_or(i.location);
                (x, y, i.scale_factor.max(1))
            }
            None => (0, 0, 1),
        }
    }

    fn realloc(&mut self, idx: usize) {
        let (_, _, scale) = self.output_geometry(&self.outs[idx].output);
        let shm = &self.shm;
        let o = &mut self.outs[idx];
        o.scale = scale;
        let (pw, ph) = ((o.w * scale) as usize, (o.h * scale) as usize);
        let mut pool = match SlotPool::new(pw * ph * 4 * 2, shm) {
            Ok(p) => p,
            Err(_) => return,
        };
        o.buffers.clear();
        for _ in 0..2 {
            // A fresh memfd is zeroed; don't touch its pages until a trail is actually drawn.
            if let Ok((b, _)) = pool.create_buffer(
                pw as i32,
                ph as i32,
                (pw * 4) as i32,
                wl_shm::Format::Argb8888,
            ) {
                o.buffers.push(b);
            }
        }
        o.pool = Some(pool);
        o.dirty = [None; 2];
        o.last = None;
        o.idx = 0;
        o.layer.wl_surface().set_buffer_scale(scale);
        o.layer.wl_surface().commit();
    }

    /// Gives the buffers' pages back to the system while nothing is drawn. The buffers are
    /// fully transparent (zeros) at that point, so dropping the pages changes nothing visible.
    fn release_memory(&mut self) {
        for o in &mut self.outs {
            if o.released || o.last.is_some() {
                continue;
            }
            if let Some(pool) = o.pool.as_mut() {
                for buf in &o.buffers {
                    // The most recently shown buffer is still "active" on the compositor side (canvas()
                    // would return None), but it is already fully transparent, so use the raw memory.
                    let map = pool.raw_data_mut(&buf.slot());
                    let start = map.as_mut_ptr() as usize;
                    let (from, to) = ((start + 4095) & !4095, (start + map.len()) & !4095);
                    if to > from {
                        // SAFETY: the range lies entirely inside the buffer mapping and is page-aligned.
                        // MADV_REMOVE zeroes shared memory, and the buffers are empty anyway
                        // (the last frame has been cleared).
                        let _ = unsafe {
                            rustix::mm::madvise(
                                from as *mut core::ffi::c_void,
                                to - from,
                                rustix::mm::Advice::LinuxRemove,
                            )
                        };
                    }
                }
            }
            let all = true;
            if all {
                o.dirty = [None; 2];
                o.released = true;
            }
        }
    }

    fn draw(&mut self, trail: &[paint::Pt], now: Instant, cfg: &Config, painter: &Painter) {
        let geos: Vec<(i32, i32, i32)> = self
            .outs
            .iter()
            .map(|o| self.output_geometry(&o.output))
            .collect();
        for (o, &(ox, oy, _)) in self.outs.iter_mut().zip(&geos) {
            if !o.configured || o.buffers.len() < 2 {
                continue;
            }
            let s = o.scale as f32;
            let (bw, bh) = (o.w * o.scale, o.h * o.scale);

            // Bounding box of the trail on this output (buffer pixels).
            // для звёзд запас должен покрывать весь полёт частицы: скорость * жизнь
            // + гравитационный дрейф, иначе dirty rect обрезает хвост разлёта
            let pad = if cfg.style == config::STYLE_STARS {
                let flight = cfg.star_speed * (1000.0 * cfg.star_life_mul) / 1000.0;
                let grav = 0.5 * 800.0 * cfg.star_gravity * (1000.0 * cfg.star_life_mul / 1000.0).powi(2);
                (flight + grav + cfg.star_spread + cfg.max_width * 4.0 + 20.0) * s
            } else {
                cfg.max_width * s * (if cfg.glow > 0.01 { 3.4 } else { 1.6 }) + 4.0
            };
            let mut cur: Option<IRect> = None;
            for p in trail {
                let (x, y) = ((p.x - ox as f32) * s, (p.y - oy as f32) * s);
                let r = IRect {
                    x0: (x - pad).floor() as i32,
                    y0: (y - pad).floor() as i32,
                    x1: (x + pad).ceil() as i32,
                    y1: (y + pad).ceil() as i32,
                };
                cur = Some(cur.map_or(r, |c| c.union(r)));
            }
            let clip = IRect {
                x0: 0,
                y0: 0,
                x1: bw,
                y1: bh,
            };
            cur = cur.and_then(|c| {
                let r = IRect {
                    x0: c.x0.max(0),
                    y0: c.y0.max(0),
                    x1: c.x1.min(clip.x1),
                    y1: c.y1.min(clip.y1),
                };
                (r.x0 < r.x1 && r.y0 < r.y1).then_some(r)
            });
            // звёзды живут дольше следа: пока есть живые частицы, кадр нужен
            let stars_alive = cfg.style == config::STYLE_STARS && painter.has_live_stars();
            if cur.is_none() && o.last.is_none() && !stars_alive {
                continue;
            }

            let i = o.idx;
            let wipe = union_opt(o.dirty[i], cur);
            let damage = union_opt(cur, o.last);
            let Some(pool) = o.pool.as_mut() else {
                continue;
            };
            let Some(canvas) = o.buffers[i].canvas(pool) else {
                continue;
            };

            if let Some(w) = wipe {
                let stride = bw as usize * 4;
                for y in w.y0..w.y1 {
                    let a = y as usize * stride + w.x0 as usize * 4;
                    let b = y as usize * stride + w.x1 as usize * 4;
                    canvas[a..b].fill(0);
                }
            }

            if cur.is_some() {
                if let Some(mut pm) = PixmapMut::from_bytes(canvas, bw as u32, bh as u32) {
                    // painter может отчитаться о реальных габаритах (звёзды улетают),
                    // расширяем dirty rect чтобы не обрезать
                    if let Some((bx0, by0, bx1, by1)) = painter.paint(&mut pm, trail, (ox, oy), s, now, cfg) {
                        let extra = IRect {
                            x0: bx0.floor() as i32,
                            y0: by0.floor() as i32,
                            x1: bx1.ceil() as i32,
                            y1: by1.ceil() as i32,
                        };
                        cur = cur.and_then(|c| {
                            let r = c.union(extra);
                            let r = IRect {
                                x0: r.x0.max(0),
                                y0: r.y0.max(0),
                                x1: r.x1.min(clip.x1),
                                y1: r.y1.min(clip.y1),
                            };
                            (r.x0 < r.x1 && r.y0 < r.y1).then_some(r)
                        });
                    }
                }
            }

            let surface = o.layer.wl_surface();
            if let Some(d) = damage {
                surface.damage_buffer(d.x0, d.y0, d.x1 - d.x0, d.y1 - d.y0);
            }
            o.buffers[i].attach_to(surface).ok();
            o.layer.commit();
            o.dirty[i] = cur;
            o.last = cur;
            o.idx ^= 1;
            o.released = false;
        }
    }
}

fn hsl(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h * 6.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (r + m, g + m, b + m)
}

// --- Hyprland IPC -----------------------------------------------------------

struct Ipc {
    path: String,
}

impl Ipc {
    fn new() -> Option<Self> {
        let rt = env::var("XDG_RUNTIME_DIR").ok()?;
        let sig = env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
        Some(Self {
            path: format!("{rt}/hypr/{sig}/.socket.sock"),
        })
    }

    // один запрос к hyprland ipc: шлёт команду, читает ответ целиком
    fn request(&self, cmd: &str) -> Option<String> {
        let mut s = UnixStream::connect(&self.path).ok()?;
        s.set_read_timeout(Some(Duration::from_millis(50))).ok()?;
        s.write_all(cmd.as_bytes()).ok()?;
        let mut buf = [0u8; 256];
        let n = s.read(&mut buf).ok()?;
        Some(String::from_utf8_lossy(&buf[..n]).into_owned())
    }

    fn cursor(&self) -> Option<(f32, f32)> {
        let txt = self.request("/cursorpos")?;
        let (x, y) = txt.trim().split_once(',')?;
        Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
    }

    // активное окно в фуллскрине? j/activewindow отвечает json'ом,
    // строку "fullscreen": 1 выцепляем без парсера, его ради одного байта тащить жалко
    fn fullscreen(&self) -> bool {
        let Some(txt) = self.request("j/activewindow") else {
            return false;
        };
        // нет активного окна (пустой ответ Invalid) — не фуллскрин
        if txt.contains("Invalid") {
            return false;
        }
        for line in txt.lines() {
            let line = line.trim().trim_end_matches(',');
            if let Some(rest) = line.strip_prefix("\"fullscreen\": ") {
                return rest.trim() != "0";
            }
        }
        false
    }
}

// --- sctk handlers ----------------------------------------------------------

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, o: wl_output::WlOutput) {
        self.add_output(qh, o);
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, o: wl_output::WlOutput) {
        let (_, _, scale) = self.output_geometry(&o);
        if let Some(idx) = self.outs.iter().position(|x| x.output == o) {
            if self.outs[idx].configured && self.outs[idx].scale != scale {
                self.realloc(idx);
            }
        }
    }
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, o: wl_output::WlOutput) {
        self.outs.retain(|x| x.output != o);
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        self.outs
            .retain(|o| o.layer.wl_surface() != layer.wl_surface());
        if self.outs.is_empty() {
            self.exit = true;
        }
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let Some(idx) = self
            .outs
            .iter()
            .position(|o| o.layer.wl_surface() == layer.wl_surface())
        else {
            return;
        };
        let (w, h) = (configure.new_size.0 as i32, configure.new_size.1 as i32);
        if w == 0 || h == 0 {
            return;
        }
        let changed = self.outs[idx].w != w || self.outs[idx].h != h || !self.outs[idx].configured;
        self.outs[idx].w = w;
        self.outs[idx].h = h;
        self.outs[idx].configured = true;
        if changed {
            self.realloc(idx);
        }
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

delegate_registry!(App);
delegate_dispatch2!(App);

// --- main -------------------------------------------------------------------

fn main() {
    let cfg = config::parse_args();
    if let Some(i) = std::env::args().position(|a| a == "--dump-png") {
        let out = std::env::args().nth(i + 1).unwrap_or_else(|| "/tmp/trail.png".into());
        paint::dump_png(&out, &cfg).expect("dump");
        println!("{out}");
        return;
    }
    let Some(ipc) = Ipc::new() else {
        eprintln!("HYPRLAND_INSTANCE_SIGNATURE is not set; run inside a Hyprland session.");
        std::process::exit(1);
    };

    let conn = Connection::connect_to_env().expect("cannot connect to the Wayland display");
    let (globals, mut queue) = registry_queue_init::<App>(&conn).expect("registry");
    let qh = queue.handle();

    let mut app = App {
        registry: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor: CompositorState::bind(&globals, &qh).expect("wl_compositor"),
        shm: Shm::bind(&globals, &qh).expect("wl_shm"),
        layer_shell: LayerShell::bind(&globals, &qh).expect("zwlr_layer_shell_v1"),
        outs: Vec::new(),
        exit: false,
    };

    let painter = Painter::new(&cfg);
    let life = Duration::from_millis(cfg.life as u64);
    let mut trail: Vec<paint::Pt> = Vec::new();
    let mut ipc_fails = 0;
    let mut idle_since: Option<Instant> = None;
    let mut last_pos: Option<(f32, f32)> = None;
    let mut fs_check: Option<Instant> = None;
    let mut fs_now = false;

    while !app.exit {
        let now = Instant::now();

        // фуллскрин дорогой запрос, дёргаем раз в 250мс а не каждый кадр
        if cfg.hide_on_fullscreen && fs_check.map_or(true, |t| now.duration_since(t).as_millis() >= 250) {
            fs_check = Some(now);
            fs_now = ipc.fullscreen();
        }

        match ipc.cursor() {
            Some((x, y)) => {
                ipc_fails = 0;
                // Compare with the last known position, not the end of the trail: once the trail has
                // expired it is empty, and a stationary cursor would keep adding points forever.
                // в фуллскрине точки не добавляем, след догорает сам
                if last_pos != Some((x, y)) && !fs_now {
                    last_pos = Some((x, y));
                    trail.push(paint::Pt { x, y, t: now });
                }
            }
            None => {
                ipc_fails += 1;
                if ipc_fails > 100 {
                    break; // Hyprland is gone
                }
            }
        }
        trail.retain(|p| now.duration_since(p.t) < life);

        // кадр нужен только если есть след или живые звёзды. иначе не дёргаем
        // оверлей вообще: пустой commit каждый цикл жжёт CPU на ровном месте
        let need_frame = !trail.is_empty()
            || (cfg.style == config::STYLE_STARS && painter.has_live_stars())
            || app.outs.iter().any(|o| o.last.is_some());
        if need_frame {
            app.draw(&trail, now, &cfg, &painter);
        }

        // Flush requests and handle events without blocking.
        let _ = queue.flush();
        if let Some(guard) = queue.prepare_read() {
            let _ = guard.read();
        }
        let _ = queue.dispatch_pending(&mut app);

        let busy = !trail.is_empty()
            || (cfg.style == config::STYLE_STARS && painter.has_live_stars())
            || app.outs.iter().any(|o| o.last.is_some());
        if busy {
            idle_since = None;
        } else if idle_since.get_or_insert(now).elapsed() > Duration::from_millis(500) {
            app.release_memory();
        }
        sleep(if busy { FAST } else { IDLE });
    }
}
