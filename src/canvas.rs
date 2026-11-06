//! The canvas itself: a transform, a list of pictures, and a grid of square
//! cards that repeats for ever in every direction.

use serde::{Deserialize, Serialize};

/// Card side in world pixels (1 world pixel = 1 terminal pixel at zoom 1).
pub const CARD: f64 = 210.0;
/// How far the view starts from the origin, in world pixels.
pub const GAP: f64 = 16.0;

pub const ZOOM_MIN: f64 = 0.25;
pub const ZOOM_MAX: f64 = 6.0;

#[derive(Clone, Serialize, Deserialize)]
pub struct Canvas {
    /// world coordinate of the viewport's top-left corner
    pub ox: f64,
    pub oy: f64,
    pub zoom: f64,
    /// pictures on this canvas, in the order they were added
    pub images: Vec<String>,
    /// folder the picker searches, when this canvas was opened with one
    pub root: Option<String>,
    /// salts the scatter pattern, so two canvases with the same pictures do
    /// not look identical
    #[serde(default)]
    pub seed: u64,
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            ox: -GAP,
            oy: -GAP,
            zoom: 1.0,
            images: Vec::new(),
            root: None,
            seed: seed_now(),
        }
    }
}

fn seed_now() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x5DEE_CE66);
    nanos ^ (std::process::id() as u64) << 32
}

impl Canvas {
    pub fn zoom_at(&mut self, factor: f64, sx: f64, sy: f64) {
        let (wx, wy) = self.to_world(sx, sy);
        self.zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        self.ox = wx - sx / self.zoom;
        self.oy = wy - sy / self.zoom;
    }

    /// Screen pixels -> world.
    pub fn to_world(&self, sx: f64, sy: f64) -> (f64, f64) {
        (self.ox + sx / self.zoom, self.oy + sy / self.zoom)
    }

    pub fn add(&mut self, path: &str, allow_duplicate: bool) -> bool {
        if !allow_duplicate && self.images.iter().any(|p| p == path) {
            return false;
        }
        self.images.push(path.to_string());
        true
    }

    pub fn remove_index(&mut self, idx: usize) -> Option<String> {
        if idx >= self.images.len() {
            return None;
        }
        Some(self.images.remove(idx))
    }

    pub fn index_of(&self, path: &str) -> Option<usize> {
        self.images.iter().position(|p| p == path)
    }

    /// Cards repeat the pictures in a diagonal pattern: one picture fills
    /// everything, two alternate like a checkerboard, more spread out.
    pub fn image_at(&self, i: i64, j: i64) -> Option<usize> {
        let n = self.images.len();
        if n == 0 {
            return None;
        }
        Some(pick_image(i, j, n, self.seed))
    }
}

/// The card grid measured in whole terminal cells.
///
/// Everything is an integer number of cells, computed once per frame from the
/// zoom level, so every card is exactly the same size and every gap exactly the
/// same width — no rounding drift towards the edges of the viewport.
pub struct Grid {
    pub card_c: i64,
    pub card_r: i64,
    pub step_c: i64,
    pub step_r: i64,
}

impl Grid {
    pub fn new(zoom: f64, cell_w: u16, cell_h: u16) -> Grid {
        let cw = cell_w.max(1) as f64;
        let ch = cell_h.max(1) as f64;
        let z = zoom.max(0.05);
        let target = CARD * z;

        // A card stays square, but it is measured in whole cells and a cell is
        // taller than it is wide (16x31 on some terminals). Rounding the two
        // axes separately makes cards 20 % off at some zoom levels, so pick the
        // row count that lands closest to the wanted size and derive the
        // columns from it.
        let mut card_c = 1i64;
        let mut card_r = 1i64;
        let mut best = f64::MAX;
        let rows = ((target / ch).round() as i64 + 1).clamp(1, 96);
        for r in 1..=rows {
            let c = ((r as f64 * ch) / cw).round().max(1.0) as i64;
            let side = (c as f64 * cw).min(r as f64 * ch);
            let err = (side - target).abs();
            if err < best {
                best = err;
                card_c = c;
                card_r = r;
            }
        }

        // No gap: the pictures are all cut square, so they tile into one sheet.
        Grid {
            card_c,
            card_r,
            step_c: card_c,
            step_r: card_r,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Cell {
    pub i: i64,
    pub j: i64,
    /// top-left corner of the whole card, may be negative
    pub x: i64,
    pub y: i64,
    /// the part of the card that is on screen
    pub vx: i64,
    pub vy: i64,
    pub vw: i64,
    pub vh: i64,
    pub image: Option<usize>,
}

impl Canvas {
    /// Every card that intersects a `cols` x `rows` cell viewport.
    pub fn cells(&self, grid: &Grid, cols: i64, rows: i64, cell_w: u16, cell_h: u16) -> Vec<Cell> {
        let mut out = Vec::new();
        if cols <= 0 || rows <= 0 || self.zoom <= 0.0 {
            return out;
        }
        let cw = cell_w.max(1) as f64;
        let ch = cell_h.max(1) as f64;
        let pan_c = (self.ox * self.zoom / cw).round() as i64;
        let pan_r = (self.oy * self.zoom / ch).round() as i64;

        let i0 = (pan_c - grid.card_c).div_euclid(grid.step_c) - 1;
        let i1 = (pan_c + cols).div_euclid(grid.step_c) + 1;
        let j0 = (pan_r - grid.card_r).div_euclid(grid.step_r) - 1;
        let j1 = (pan_r + rows).div_euclid(grid.step_r) + 1;

        let n = self.images.len();
        for j in j0..=j1 {
            let y = j * grid.step_r - pan_r;
            if y >= rows || y + grid.card_r <= 0 {
                continue;
            }
            for i in i0..=i1 {
                let x = i * grid.step_c - pan_c;
                if x >= cols || x + grid.card_c <= 0 {
                    continue;
                }
                let vx = x.max(0);
                let vy = y.max(0);
                let vw = (x + grid.card_c).min(cols) - vx;
                let vh = (y + grid.card_r).min(rows) - vy;
                if vw < 1 || vh < 1 {
                    continue;
                }
                out.push(Cell {
                    i,
                    j,
                    x,
                    y,
                    vx,
                    vy,
                    vw,
                    vh,
                    image: if n == 0 {
                        None
                    } else {
                        Some(pick_image(i, j, n, self.seed))
                    },
                });
            }
        }
        out
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    pub tabs: Vec<Canvas>,
    pub active: usize,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            tabs: vec![Canvas::default()],
            active: 0,
        }
    }
}

impl Session {
    pub fn tab(&self) -> &Canvas {
        &self.tabs[self.active.min(self.tabs.len() - 1)]
    }

    pub fn tab_mut(&mut self) -> &mut Canvas {
        let i = self.active.min(self.tabs.len() - 1);
        &mut self.tabs[i]
    }
}

/// Which picture lands on a card.
///
/// Pure per-cell noise clumps — with six pictures you would keep meeting the
/// same one twice in a row. Instead the plane is cut into 4x4 blocks and each
/// block gets its own shuffled bag of pictures, so any small patch holds a
/// fair mix while the arrangement still looks random. The shuffle comes from a
/// hash of the block, so a card keeps its picture however far you drag.
fn pick_image(i: i64, j: i64, n: usize, seed: u64) -> usize {
    if n <= 1 {
        return 0;
    }
    const B: i64 = 5;
    const SLOTS: usize = (B * B) as usize;
    let (bi, bj) = (i.div_euclid(B), j.div_euclid(B));
    let slot = (j.rem_euclid(B) * B + i.rem_euclid(B)) as usize;

    let mut bag: [u8; SLOTS] = [0; SLOTS];
    for (k, o) in bag.iter_mut().enumerate() {
        *o = k as u8;
    }
    let mut h = hash2(bi, bj, seed);
    for k in (1..SLOTS).rev() {
        h = mix(h);
        bag.swap(k, (h % (k as u64 + 1)) as usize);
    }

    // Ranks spread evenly over the pictures, plus a per-block offset so picture
    // sets larger than a block still cycle through all of them.
    let offset = (hash2(bi, bj, seed ^ 0x9E37_79B9) % n as u64) as usize;
    let mut pic = [0usize; SLOTS];
    for k in 0..SLOTS {
        pic[k] = (bag[k] as usize * n / SLOTS + offset) % n;
    }
    // A shuffled bag still lets neighbours collide by chance; nudge those onto
    // a later slot so no two touching cards show the same picture.
    for k in 0..SLOTS {
        let left = if k % B as usize == 0 { None } else { Some(pic[k - 1]) };
        let up = if k < B as usize { None } else { Some(pic[k - B as usize]) };
        if left == Some(pic[k]) || up == Some(pic[k]) {
            for t in (k + 1)..SLOTS {
                let p = pic[t];
                if p != pic[k] && left != Some(p) && up != Some(p) {
                    pic.swap(k, t);
                    break;
                }
            }
        }
    }
    pic[slot]
}

fn mix(mut h: u64) -> u64 {
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^ (h >> 33)
}

fn hash2(i: i64, j: i64, seed: u64) -> u64 {
    let mut h = seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= (j as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 31;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^ (h >> 33)
}
