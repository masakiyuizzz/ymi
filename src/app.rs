use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::i18n;
use crate::canvas::{Canvas, Grid, Session, ZOOM_MAX, ZOOM_MIN};
use crate::cli::Args;
use crate::daemon;
use crate::graphics::{Graphics, Placement, Proto, Rgb};
use crate::scan;
use crate::term::Caps;
use crate::theme::Theme;

/// Pictures are decoded at twice the card side: sharp at 100 %, acceptable up
/// to 200 % zoom without keeping the originals in memory.
const THUMB: u32 = (crate::canvas::CARD * 2.0) as u32;
const CACHE_LIMIT: usize = 96 << 20;

/// Placement id reserved for the zoomed picture in the modal.
const MODAL_PID: u32 = 0xFFFF_FFFE;
/// Placement id reserved for the preview pane of the picker.
const PREVIEW_PID: u32 = 0xFFFF_FFFD;

/// Cards go under the text, and below `i32::MIN / 2` they also go under cells
/// with a non-default background — which is what makes an opaque dialog panel
/// cover them. Pictures that belong *inside* a panel (the picker preview, the
/// full-size view) keep `Z_ABOVE` so the panel's own background cannot hide them.
const Z_UNDER: i32 = -1_100_000_000;
const Z_ABOVE: i32 = 0;

#[derive(Clone)]
pub struct TileView {
    pub i: i64,
    pub j: i64,
    /// the visible part of the card, in screen cells
    pub rect: Rect,
    /// the whole card, in screen pixels (x, y, w, h)
    pub box_px: (f64, f64, f64, f64),
    pub image: Option<String>,
}

pub enum Modal {
    Help,
    Image(String),
    Picker,
    Input,
}

pub struct Picker {
    pub root: String,
    pub all: Vec<String>,
    pub query: String,
    pub sel: usize,
}

struct Drag {
    last: (u16, u16),
    moved: f64,
    /// world pixels per second, smoothed
    vel: (f64, f64),
    at: Instant,
}

/// Glide after a drag or a wheel notch. The canvas keeps moving while the
/// speed decays exponentially, so a flick of the mouse coasts to a stop.
#[derive(Clone, Copy)]
struct Momentum {
    /// world pixels per second
    v: (f64, f64),
    /// decay rate; bigger stops sooner
    k: f64,
    at: Instant,
}

pub struct App {
    pub session: Session,
    pub gfx: Graphics,
    pub caps: Caps,
    pub theme: Theme,
    client: daemon::Client,

    pub content: Rect,
    pub tabs_row: Option<Rect>,
    pub status_row: Rect,
    pub tiles: Vec<TileView>,
    pub hover: Option<(i64, i64)>,
    drag: Option<Drag>,
    momentum: Option<Momentum>,

    pub modal: Option<Modal>,
    pub input: String,
    pub completions: Vec<String>,
    pub picker: Option<Picker>,
    /// picture shown on the right of the picker
    pub preview: Option<String>,
    preview_at: Instant,

    pub toast: Option<(String, Instant)>,
    pub quit: bool,
    hard_quit: bool,

    cache: HashMap<String, Arc<Rgb>>,
    cache_bytes: usize,
    dirty: bool,
    unsaved: bool,
    last_save: Instant,
}

impl App {
    pub fn new(args: &Args, caps: Caps, session: Session, client: daemon::Client) -> Result<App> {
        let proto = if caps.kitty { Proto::Kitty } else { Proto::None };
        let theme = Theme::from_light(caps.light);
        let mut app = App {
            session,
            gfx: Graphics::new(proto),
            caps,
            theme,
            client,
            content: Rect::default(),
            tabs_row: None,
            status_row: Rect::default(),
            tiles: Vec::new(),
            hover: None,
            drag: None,
            momentum: None,
            modal: None,
            input: String::new(),
            completions: Vec::new(),
            picker: None,
            preview: None,
            preview_at: Instant::now(),
            toast: None,
            quit: false,
            hard_quit: false,
            cache: HashMap::new(),
            cache_bytes: 0,
            dirty: true,
            unsaved: true,
            last_save: Instant::now(),
        };
        app.open_target(&args.target);
        Ok(app)
    }

    fn open_target(&mut self, target: &str) {
        let s = target.trim();
        if s.is_empty() {
            return;
        }
        let path = scan::expand(s);
        if path.is_dir() {
            let root = path.display().to_string();
            self.session.tab_mut().root = Some(root);
            self.toast(i18n::t("目录已就绪：a 或点 + 选图片", "folder ready: press a or click + to pick pictures"));
        } else if path.is_file() {
            self.add_image(&path.display().to_string(), true);
        } else {
            self.toast(i18n::p(format!("找不到 {s}"), format!("cannot find {s}")));
        }
    }

    fn add_image(&mut self, path: &str, allow_duplicate: bool) {
        let added = self.session.tab_mut().add(path, allow_duplicate);
        if added {
            let n = self.session.tab().images.len();
            let name = Path::new(path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string());
            self.toast(i18n::p(
            format!("已加入 {name}（{n} 张）"),
            format!("added {name} ({n} on the canvas)"),
        ));
            self.dirty = true;
        } else {
            self.toast(i18n::t("这张已经在画布上了", "that one is already on the canvas"));
        }
    }

    fn tab(&self) -> &Canvas {
        self.session.tab()
    }

    fn tab_mut(&mut self) -> &mut Canvas {
        self.session.tab_mut()
    }

    fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    pub fn run(&mut self) -> Result<()> {
        let backend = CrosstermBackend::new(std::io::stdout());
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;
        let _ = terminal.hide_cursor();
        // A previous run (or a crashed one) may have left placements behind
        // that we know nothing about; the ids are gone, so wipe the window.
        if self.gfx.usable() {
            let mut out = std::io::stdout();
            use std::io::Write as _;
            let _ = out.write_all(b"\x1b_Ga=d,d=A\x1b\\");
            let _ = out.flush();
        }

        let res = self.event_loop(&mut terminal);

        let mut out = std::io::stdout();
        let _ = self.gfx.clear(&mut out);
        if self.hard_quit {
            let _ = self.client.quit();
        }
        res
    }

    fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        let mut last_size = (0u16, 0u16);
        loop {
            if self.dirty || last_size != terminal.size().map(|s| (s.width, s.height)).unwrap_or((0, 0)) {
                last_size = terminal.size().map(|s| (s.width, s.height)).unwrap_or((0, 0));
                self.dirty = false;
                let area = Rect {
                    x: 0,
                    y: 0,
                    width: last_size.0,
                    height: last_size.1,
                };
                self.prepare(area);
                terminal.draw(|f| crate::ui::draw(f, self))?;

                let rect = self.content;
                if self.gfx.usable() && rect.width > 2 && rect.height > 2 {
                    let mut out = std::io::stdout();
                    self.sync_graphics(&mut out, rect)?;
                }
            }

            if self.quit {
                let _ = self.client.save(&self.session);
                return Ok(());
            }

            let wait = if self.momentum.is_some() {
                8
            } else if matches!(self.modal, Some(Modal::Picker)) {
                24
            } else {
                60
            };
            if event::poll(Duration::from_millis(wait))? {
                let ev = event::read()?;
                self.handle_event(ev);
                if self.dirty {
                    self.unsaved = true;
                }
            }
            self.tick();

            if let Some((_, at)) = &self.toast {
                if at.elapsed() > Duration::from_secs(2) {
                    self.toast = None;
                    self.dirty = true;
                }
            }
            if self.unsaved && self.last_save.elapsed() > Duration::from_millis(250) {
                self.last_save = Instant::now();
                self.unsaved = false;
                let _ = self.client.save(&self.session);
            }
        }
    }

    fn handle_event(&mut self, ev: Event) {
        match ev {
            Event::Key(k) => self.on_key(k),
            Event::Mouse(m) => self.on_mouse(m),
            Event::Resize(_, _) => {
                self.gfx.clear(&mut std::io::stdout()).ok();
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if std::env::var_os("YMI_DEBUG").is_some() {
            use std::io::Write as _;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/ymi-keys.log")
            {
                let _ = writeln!(f, "{key:?} ctrl={ctrl}");
            }
        }

        // modal first
        if let Some(modal) = &self.modal {
            match modal {
                Modal::Help => {
                    self.modal = None;
                    self.dirty = true;
                    return;
                }
                Modal::Image(path) => {
                    let path = path.clone();
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                            self.modal = None;
                            self.dirty = true;
                        }
                        KeyCode::Delete | KeyCode::Char('x') => {
                            if let Some(idx) = self.session.tab().index_of(&path) {
                                self.session.tab_mut().remove_index(idx);
                                self.toast(i18n::t("已移除这张", "removed"));
                            }
                            self.modal = None;
                            self.dirty = true;
                        }
                        _ => {}
                    }
                    return;
                }
                Modal::Picker => {
                    let mut close = false;
                    if let Some(p) = self.picker.as_mut() {
                        match key.code {
                            KeyCode::Esc => close = true,
                            KeyCode::Up => p.sel = p.sel.saturating_sub(1),
                            KeyCode::Down => {
                                let n = scan::filter(&p.all, &p.query).len();
                                if p.sel + 1 < n {
                                    p.sel += 1;
                                }
                            }
                            KeyCode::Backspace => {
                                p.query.pop();
                                p.sel = 0;
                            }
                            KeyCode::Char(c) if !ctrl => {
                                p.query.push(c);
                                p.sel = 0;
                            }
                            KeyCode::Enter => {
                                let hits = scan::filter(&p.all, &p.query);
                                if let Some(&idx) = hits.get(p.sel) {
                                    let path = p.all[idx].clone();
                                    // Keep the list open: adding one picture at a
                                    // time means re-opening it after every pick.
                                    self.add_image(&path, false);
                                }
                            }
                            _ => {}
                        }
                    }
                    self.preview_at = Instant::now();
                    if close {
                        self.modal = None;
                        self.picker = None;
                        self.preview = None;
                    }
                    self.dirty = true;
                    return;
                }
                Modal::Input => {}
            }
        }

        if self.modal.is_some() {
            // path input
            match key.code {
                KeyCode::Esc => {
                    self.modal = None;
                    self.completions.clear();
                }
                KeyCode::Backspace => {
                    self.input.pop();
                    self.completions.clear();
                }
                KeyCode::Tab => {
                    let c = scan::complete(&self.input);
                    self.input = c.filled;
                    self.completions = c.options;
                }
                KeyCode::Enter => {
                    let text = self.input.clone();
                    self.submit_input(&text);
                }
                KeyCode::Char(c) if !ctrl => {
                    self.input.push(c);
                    self.completions.clear();
                }
                _ => {}
            }
            self.dirty = true;
            return;
        }

        if ctrl && key.code == KeyCode::Char('q') {
            self.hard_quit = true;
            self.quit = true;
            return;
        }

        match key.code {
            KeyCode::Esc => {
                self.quit = true;
                return;
            }
            KeyCode::Char('?') => {
                self.modal = Some(Modal::Help);
                self.dirty = true;
                return;
            }
            KeyCode::Char('a') | KeyCode::Char('+') if !ctrl => {
                self.open_add();
                return;
            }
            KeyCode::Delete | KeyCode::Char('x') if !ctrl => {
                self.delete_hovered();
                return;
            }
            KeyCode::Char('t') if ctrl => {
                self.session.tabs.push(Canvas::default());
                self.session.active = self.session.tabs.len() - 1;
                self.hover = None;
                self.dirty = true;
                return;
            }
            KeyCode::Char('w') if ctrl => {
                self.close_tab();
                return;
            }
            KeyCode::Char('=') | KeyCode::Char('+') if ctrl => {
                self.zoom(1.1);
                return;
            }
            KeyCode::Char('-') if ctrl => {
                self.zoom(1.0 / 1.1);
                return;
            }
            KeyCode::Char('0') if ctrl => {
                self.tab_mut().zoom = 1.0;
                self.dirty = true;
                return;
            }
            KeyCode::Left if ctrl => {
                self.switch_tab(-1);
                return;
            }
            KeyCode::Right if ctrl => {
                self.switch_tab(1);
                return;
            }
            KeyCode::Char(c @ '1'..='9') if ctrl => {
                let idx = c as usize - '1' as usize;
                if idx < self.session.tabs.len() {
                    self.set_tab(idx);
                }
                return;
            }
            KeyCode::Left => self.pan(40.0, 0.0),
            KeyCode::Right => self.pan(-40.0, 0.0),
            KeyCode::Up => self.pan(0.0, 40.0),
            KeyCode::Down => self.pan(0.0, -40.0),
            _ => {}
        }
    }

    fn submit_input(&mut self, text: &str) {
        let path = scan::expand(text);
        if path.is_dir() {
            let root = path.display().to_string();
            self.tab_mut().root = Some(root);
            self.modal = None;
            self.completions.clear();
            self.open_picker();
        } else if path.is_file() {
            let p = path.display().to_string();
            self.modal = None;
            self.completions.clear();
            self.add_image(&p, true);
        } else {
            self.toast(i18n::p(
                format!("找不到 {text}"),
                format!("cannot find {text}"),
            ));
        }
        self.dirty = true;
    }

    fn open_input(&mut self) {
        self.modal = Some(Modal::Input);
        self.input = self.tab().root.clone().unwrap_or_default();
        if !self.input.is_empty() && !self.input.ends_with('/') {
            self.input.push('/');
        }
        self.completions.clear();
        self.dirty = true;
    }

    /// `a`: with a folder at hand this is the picture list, otherwise a path.
    fn open_add(&mut self) {
        if self.tab().root.is_some() {
            self.open_picker();
        } else {
            self.open_input();
        }
    }

    fn open_picker(&mut self) {
        let Some(root) = self.tab().root.clone() else {
            self.open_input();
            return;
        };
        let all = scan::scan(Path::new(&root));
        if all.is_empty() {
            self.toast(i18n::t("这个目录里没有图片", "no pictures in this folder"));
            return;
        }
        self.picker = Some(Picker {
            root,
            all,
            query: String::new(),
            sel: 0,
        });
        self.modal = Some(Modal::Picker);
        self.dirty = true;
    }

    fn delete_hovered(&mut self) {
        let Some((i, j)) = self.hover else {
            self.toast(i18n::t("把鼠标移到一张图上再删", "point at a picture first, then remove it"));
            return;
        };
        let Some(idx) = self.tab().image_at(i, j) else {
            self.toast(i18n::t("这张是空卡片", "that card is empty"));
            return;
        };
        if let Some(path) = self.tab_mut().remove_index(idx) {
            let name = Path::new(&path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or(path);
            self.toast(i18n::p(
            format!("已移除 {name}"),
            format!("removed {name}"),
        ));
            self.dirty = true;
        }
    }

    fn close_tab(&mut self) {
        if self.session.tabs.len() <= 1 {
            self.toast(i18n::t("最后一页了", "that is the last canvas"));
            return;
        }
        self.session.tabs.remove(self.session.active);
        if self.session.active >= self.session.tabs.len() {
            self.session.active = self.session.tabs.len() - 1;
        }
        self.hover = None;
        self.dirty = true;
    }

    fn switch_tab(&mut self, delta: i64) {
        let n = self.session.tabs.len() as i64;
        let mut idx = self.session.active as i64 + delta;
        if idx < 0 {
            idx = n - 1;
        }
        if idx >= n {
            idx = 0;
        }
        self.set_tab(idx as usize);
    }

    fn set_tab(&mut self, idx: usize) {
        self.session.active = idx;
        self.hover = None;
        self.dirty = true;
    }

    fn zoom(&mut self, factor: f64) {
        let (cx, cy) = (
            self.content.width as f64 * self.caps.cell_w as f64 / 2.0,
            self.content.height as f64 * self.caps.cell_h as f64 / 2.0,
        );
        let cur = self.tab().zoom;
        let z = (cur * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        self.tab_mut().zoom_at(z / cur, cx, cy);
        self.dirty = true;
    }

    fn pan(&mut self, dx: f64, dy: f64) {
        self.tab_mut().ox += dx;
        self.tab_mut().oy += dy;
        self.dirty = true;
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        if self.modal.is_some() {
            if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
                match &self.modal {
                    Some(Modal::Help) | Some(Modal::Image(_)) => {
                        self.modal = None;
                        self.dirty = true;
                    }
                    _ => {}
                }
            }
            return;
        }

        let inside = self.content.width > 0
            && m.column >= self.content.x
            && m.column < self.content.x + self.content.width
            && m.row >= self.content.y
            && m.row < self.content.y + self.content.height;

        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if !inside {
                    return;
                }
                self.momentum = None;
                self.drag = Some(Drag {
                    last: (m.column, m.row),
                    moved: 0.0,
                    vel: (0.0, 0.0),
                    at: Instant::now(),
                });
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                let z = self.tab().zoom;
                let cw = self.caps.cell_w as f64;
                let ch = self.caps.cell_h as f64;
                let Some(drag) = self.drag.as_mut() else { return };
                let dx = m.column as f64 - drag.last.0 as f64;
                let dy = m.row as f64 - drag.last.1 as f64;
                drag.last = (m.column, m.row);
                drag.moved += dx.abs() + dy.abs();
                if drag.moved <= 1.0 {
                    return;
                }
                let wx = -dx * cw / z;
                let wy = -dy * ch / z;
                let now = Instant::now();
                let dt = now.duration_since(drag.at).as_secs_f64().clamp(0.006, 0.12);
                drag.vel.0 = 0.65 * drag.vel.0 + 0.35 * (wx / dt);
                drag.vel.1 = 0.65 * drag.vel.1 + 0.35 * (wy / dt);
                drag.at = now;
                self.tab_mut().ox += wx;
                self.tab_mut().oy += wy;
                self.dirty = true;
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let Some(drag) = self.drag.take() else { return };
                if drag.moved > 2.0 {
                    // Let go while moving: carry on at the speed of the hand.
                    let speed = drag.vel.0.abs() + drag.vel.1.abs();
                    if speed > 150.0 && drag.at.elapsed() < Duration::from_millis(140) {
                        self.momentum = Some(Momentum {
                            v: clamp_speed(drag.vel),
                            k: 3.4,
                            at: Instant::now(),
                        });
                    }
                    return;
                }
                if inside {
                    self.click(m.column, m.row);
                }
            }
            MouseEventKind::ScrollDown => self.scroll(0.0, 3.0),
            MouseEventKind::ScrollUp => self.scroll(0.0, -3.0),
            MouseEventKind::ScrollLeft => self.scroll(-3.0, 0.0),
            MouseEventKind::ScrollRight => self.scroll(3.0, 0.0),
            MouseEventKind::Moved => self.update_hover(m.column, m.row),
            _ => {}
        }
    }

    /// A wheel notch is an impulse rather than a jump: the canvas travels the
    /// same distance but eases out, which is what makes a mouse wheel feel like
    /// a trackpad.
    fn scroll(&mut self, dx_cells: f64, dy_cells: f64) {
        let z = self.tab().zoom;
        const K: f64 = 16.0;
        self.impulse((
            dx_cells * self.caps.cell_w as f64 / z * K,
            dy_cells * self.caps.cell_h as f64 / z * K,
        ));
    }

    fn impulse(&mut self, v: (f64, f64)) {
        let base = self.momentum.map(|m| m.v).unwrap_or((0.0, 0.0));
        self.momentum = Some(Momentum {
            v: clamp_speed((base.0 + v.0, base.1 + v.1)),
            k: 16.0,
            at: Instant::now(),
        });
        self.dirty = true;
    }

    /// One animation step: glide the canvas, refresh the picker preview.
    fn tick(&mut self) {
        if let Some(m) = self.momentum {
            let dt = m.at.elapsed().as_secs_f64().min(0.05);
            if dt > 0.001 {
                self.tab_mut().ox += m.v.0 * dt;
                self.tab_mut().oy += m.v.1 * dt;
                let decay = (-m.k * dt).exp();
                let v = (m.v.0 * decay, m.v.1 * decay);
                self.dirty = true;
                self.momentum = if v.0.abs() + v.1.abs() < 12.0 {
                    None
                } else {
                    Some(Momentum {
                        v,
                        k: m.k,
                        at: Instant::now(),
                    })
                };
            }
        }

        // The preview waits for the cursor to settle: decoding on every step of
        // a held-down arrow key would stall the list.
        match self.modal {
            Some(Modal::Picker) => {
                let pick = self.picker.as_ref().and_then(|p| {
                    let hits = scan::filter(&p.all, &p.query);
                    hits.get(p.sel).map(|&i| p.all[i].clone())
                });
                match pick {
                    Some(path) => {
                        if self.preview.as_deref() != Some(path.as_str())
                            && self.preview_at.elapsed() > Duration::from_millis(70)
                        {
                            self.preview = Some(path);
                            self.dirty = true;
                        }
                    }
                    None => {
                        if self.preview.take().is_some() {
                            self.dirty = true;
                        }
                    }
                }
            }
            _ => {
                if self.preview.take().is_some() {
                    self.dirty = true;
                }
            }
        }
    }

    /// The card under a screen cell, using the rectangles that were actually
    /// drawn — so a click always lands on the picture the eye is pointing at.
    fn tile_at(&self, col: u16, row: u16) -> Option<&TileView> {
        self.tiles.iter().find(|t| {
            col >= t.rect.x
                && col < t.rect.x + t.rect.width
                && row >= t.rect.y
                && row < t.rect.y + t.rect.height
        })
    }

    fn update_hover(&mut self, col: u16, row: u16) {
        let tile = self.tile_at(col, row).map(|t| (t.i, t.j));
        if self.hover != tile {
            self.hover = tile;
            self.dirty = true;
        }
    }

    fn click(&mut self, col: u16, row: u16) {
        let hit = self
            .tile_at(col, row)
            .map(|t| (t.i, t.j, t.image.clone()));
        match hit {
            Some((_, _, Some(path))) => {
                self.modal = Some(Modal::Image(path));
                self.dirty = true;
            }
            Some(_) => self.open_picker(),
            None => {}
        }
    }

    /// Decode a picture the first time it becomes visible.
    fn rgb_for(&mut self, path: &str) -> Option<Arc<Rgb>> {
        if let Some(rgb) = self.cache.get(path) {
            return Some(rgb.clone());
        }
        let img = image::open(path).ok()?;
        let rgb = img.to_rgb8();
        let (w, h) = rgb.dimensions();
        // The cards crop to a square, so it is the *short* side that has to
        // reach THUMB for the picture to stay sharp.
        let scale = (THUMB as f32 / w.min(h) as f32).min(1.0);
        let (tw, th) = (
            ((w as f32 * scale).round() as u32).max(1),
            ((h as f32 * scale).round() as u32).max(1),
        );
        let small = if (tw, th) == (w, h) {
            rgb
        } else {
            image::imageops::resize(&rgb, tw, th, image::imageops::FilterType::Triangle)
        };
        let data = small.into_raw();
        if self.cache_bytes + data.len() > CACHE_LIMIT {
            self.cache.clear();
            self.cache_bytes = 0;
        }
        let rgb = Arc::new(Rgb {
            w: tw,
            h: th,
            data,
        });
        self.cache_bytes += rgb.data.len();
        self.cache.insert(path.to_string(), rgb.clone());
        Some(rgb)
    }

    /// Centre square of a picture, cached. Cards are square, so handing the
    /// terminal a square source means the crop never depends on it honouring
    /// the source-rectangle keys.
    fn square_for(&mut self, path: &str) -> Option<Arc<Rgb>> {
        let key = square_key(path);
        if let Some(rgb) = self.cache.get(&key) {
            return Some(rgb.clone());
        }
        let full = self.rgb_for(path)?;
        let side = full.w.min(full.h);
        let (x0, y0) = ((full.w - side) / 2, (full.h - side) / 2);
        let mut data = Vec::with_capacity((side * side * 3) as usize);
        for row in y0..y0 + side {
            let start = ((row * full.w + x0) * 3) as usize;
            data.extend_from_slice(&full.data[start..start + (side * 3) as usize]);
        }
        let rgb = Arc::new(Rgb {
            w: side,
            h: side,
            data,
        });
        if self.cache_bytes + rgb.data.len() > CACHE_LIMIT {
            self.cache.clear();
            self.cache_bytes = 0;
            self.cache.insert(path.to_string(), full);
        }
        self.cache_bytes += rgb.data.len();
        self.cache.insert(key, rgb.clone());
        Some(rgb)
    }

    /// Work out the chrome rows and the cards that are on screen.
    fn prepare(&mut self, area: Rect) {
        self.tabs_row = None;
        self.status_row = Rect::default();
        self.content = Rect::default();
        self.tiles.clear();
        if area.width < 12 || area.height < 6 {
            return;
        }
        let inner = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        };
        let mut top = inner.y;
        if self.session.tabs.len() > 1 {
            self.tabs_row = Some(Rect {
                x: inner.x,
                y: top,
                width: inner.width,
                height: 1,
            });
            top += 1;
        }
        let status_h = 1u16;
        let content_h = inner.height.saturating_sub(top - inner.y + status_h);
        self.status_row = Rect {
            x: inner.x,
            y: top + content_h,
            width: inner.width,
            height: status_h,
        };
        self.content = Rect {
            x: inner.x,
            y: top,
            width: inner.width,
            height: content_h,
        };
        if self.content.height < 2 {
            return;
        }

        let cw = self.caps.cell_w as f64;
        let ch = self.caps.cell_h as f64;
        let cols = self.content.width as i64;
        let rows = self.content.height as i64;
        let grid = Grid::new(self.tab().zoom, self.caps.cell_w, self.caps.cell_h);
        let images = self.tab().images.clone();

        for c in self.tab().cells(&grid, cols, rows, self.caps.cell_w, self.caps.cell_h) {
            let box_px = (
                (self.content.x as i64 + c.x) as f64 * cw,
                (self.content.y as i64 + c.y) as f64 * ch,
                grid.card_c as f64 * cw,
                grid.card_r as f64 * ch,
            );
            self.tiles.push(TileView {
                i: c.i,
                j: c.j,
                rect: Rect {
                    x: self.content.x + c.vx as u16,
                    y: self.content.y + c.vy as u16,
                    width: c.vw as u16,
                    height: c.vh as u16,
                },
                box_px,
                image: c.image.and_then(|idx| images.get(idx).cloned()),
            });
        }
    }

    fn sync_graphics(&mut self, out: &mut impl std::io::Write, content: Rect) -> Result<()> {
        let cw = self.caps.cell_w as f64;
        let ch = self.caps.cell_h as f64;
        let clip = (
            content.x as f64 * cw,
            content.y as f64 * ch,
            content.width as f64 * cw,
            content.height as f64 * ch,
        );
        let mut want: Vec<Placement> = Vec::new();

        // The canvas is placed whether or not a dialog is open: cards sit below
        // the text, so a dialog simply draws over them.
        {
            let views = self.tiles.clone();
            for view in &views {
                let Some(path) = &view.image else { continue };
                let Some(rgb) = self.square_for(path) else { continue };
                let key = square_key(path);
                let _ = self.gfx.ensure(out, &key, &rgb);
                let (bx, by, bw, bh) = view.box_px;
                if let Some((x, y, c, r, crop)) =
                    place(&rgb, (bx, by, bw, bh), clip, cw, ch, Fit::Cover)
                {
                    want.push(Placement {
                        pid: placement_id(view.i, view.j),
                        path: key,
                        x,
                        y,
                        c,
                        r,
                        crop,
                        z: Z_UNDER,
                    });
                }
            }
        }

        if let Some(Modal::Image(path)) = &self.modal {
            let path = path.clone();
            if let Some(rgb) = self.rgb_for(&path) {
                let _ = self.gfx.ensure(out, &path, &rgb);
                let area = crate::ui::modal_image_rect(content, self.caps.cell_w, self.caps.cell_h);
                let bx = area.x as f64 * cw;
                let by = area.y as f64 * ch;
                if let Some((x, y, c, r, _)) = place(
                    &rgb,
                    (bx, by, area.width as f64 * cw, area.height as f64 * ch),
                    clip,
                    cw,
                    ch,
                    Fit::Contain,
                ) {
                    want.push(Placement {
                        pid: MODAL_PID,
                        path,
                        x,
                        y,
                        c,
                        r,
                        crop: None,
                        z: Z_ABOVE,
                    });
                }
            }
        }

        // Live preview of the highlighted file, on the right of the list.
        if matches!(self.modal, Some(Modal::Picker)) {
            let pane = crate::ui::picker_layout(content).preview;
            if pane.width > 2 && pane.height > 2 {
                if let Some(path) = self.preview.clone() {
                    if let Some(rgb) = self.rgb_for(&path) {
                        let _ = self.gfx.ensure(out, &path, &rgb);
                        let (bx, by, bw, bh) = (
                            (pane.x as f64 + 1.0) * cw,
                            (pane.y as f64 + 1.0) * ch,
                            (pane.width as f64 - 2.0) * cw,
                            (pane.height as f64 - 2.0) * ch,
                        );
                        if let Some((x, y, c, r, crop)) =
                            place(&rgb, (bx, by, bw, bh), clip, cw, ch, Fit::Contain)
                        {
                            want.push(Placement {
                                pid: PREVIEW_PID,
                                path,
                                x,
                                y,
                                c,
                                r,
                                crop,
                                z: Z_ABOVE,
                            });
                        }
                    }
                }
            }
        }

        self.gfx.sync(out, &want)?;
        Ok(())
    }
}

/// A fling should not turn into a teleport.
fn clamp_speed(v: (f64, f64)) -> (f64, f64) {
    const MAX: f64 = 9000.0;
    (v.0.clamp(-MAX, MAX), v.1.clamp(-MAX, MAX))
}

fn square_key(path: &str) -> String {
    format!("{path}\u{1}sq")
}

fn placement_id(i: i64, j: i64) -> u32 {
    let a = i.rem_euclid(4096) as u32;
    let b = j.rem_euclid(4096) as u32;
    (a << 12 | b) + 1
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fit {
    /// fill the box, cropping what does not fit
    Cover,
    /// fit inside the box, leaving margins
    Contain,
}

/// Where to put a picture so that it fills `box` while keeping its proportions,
/// as a cell rectangle plus the part of the source that is left over.
///
/// The picture is centred in the box, clipped to `clip` (the part of the screen
/// the canvas owns) and then snapped to whole cells. When it hangs over the
/// edge of the viewport the source is cropped instead of being scaled down, so
/// a half visible card looks like a card half out of frame and never paints
/// over the border.
fn place(
    rgb: &Rgb,
    bx: (f64, f64, f64, f64),
    clip: (f64, f64, f64, f64),
    cw: f64,
    ch: f64,
    fit: Fit,
) -> Option<(u16, u16, u16, u16, Option<(u32, u32, u32, u32)>)> {
    let (bx, by, bw, bh) = bx;
    if rgb.w == 0 || rgb.h == 0 || bw < 1.0 || bh < 1.0 || cw < 1.0 || ch < 1.0 {
        return None;
    }
    let iw = rgb.w as f64;
    let ih = rgb.h as f64;

    // Cards are filled edge to edge and the overflow is cropped away; popups
    // show the whole picture.
    let s = match fit {
        Fit::Cover => (bw / iw).max(bh / ih),
        Fit::Contain => (bw / iw).min(bh / ih),
    };
    let pw = iw * s;
    let ph = ih * s;
    let px = bx + (bw - pw) / 2.0;
    let py = by + (bh - ph) / 2.0;

    // the part of it that is on screen: inside the card *and* inside the canvas
    let x0 = px.max(bx).max(clip.0);
    let y0 = py.max(by).max(clip.1);
    let x1 = (px + pw).min(bx + bw).min(clip.0 + clip.2);
    let y1 = (py + ph).min(by + bh).min(clip.1 + clip.3);
    if x1 - x0 < 1.0 || y1 - y0 < 1.0 {
        return None;
    }

    let left = ((clip.0 / cw).round() as i64).max(0);
    let top = ((clip.1 / ch).round() as i64).max(0);
    let right = ((clip.0 + clip.2) / cw).round() as i64;
    let bottom = ((clip.1 + clip.3) / ch).round() as i64;

    let sx0 = ((x0 / cw).round() as i64).clamp(left, right - 1);
    let sy0 = ((y0 / ch).round() as i64).clamp(top, bottom - 1);
    let sx1 = ((x1 / cw).round() as i64).clamp(sx0 + 1, right);
    let sy1 = ((y1 / ch).round() as i64).clamp(sy0 + 1, bottom);

    let c = (sx1 - sx0) as u16;
    let r = (sy1 - sy0) as u16;
    let x = sx0 as u16;
    let y = sy0 as u16;

    let whole = x0 <= px + 0.01 && y0 <= py + 0.01 && x1 >= px + pw - 0.01 && y1 >= py + ph - 0.01;
    let crop = if whole {
        None
    } else {
        let fx0 = (((x0 - px) / s).round() as i64).clamp(0, rgb.w as i64);
        let fy0 = (((y0 - py) / s).round() as i64).clamp(0, rgb.h as i64);
        let fx1 = (((x1 - px) / s).round() as i64).clamp(0, rgb.w as i64);
        let fy1 = (((y1 - py) / s).round() as i64).clamp(0, rgb.h as i64);
        if fx1 - fx0 < 1 || fy1 - fy0 < 1 {
            return None;
        }
        Some((
            fx0 as u32,
            fy0 as u32,
            (fx1 - fx0) as u32,
            (fy1 - fy0) as u32,
        ))
    };

    Some((x, y, c, r, crop))
}
