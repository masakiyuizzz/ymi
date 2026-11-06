//! Painting pictures with the kitty graphics protocol.
//!
//! Each picture is uploaded once into shared memory and then placed as many
//! times as there are cards showing it. Moving the canvas only re-places the
//! tiles; no pixels travel again.
//!
//! A placement is identified by the pair (image id, placement id) — deleting a
//! single one has to name *both*, which is what `delete` does below. Getting
//! this wrong leaves every old picture on screen for ever, which looks exactly
//! like "the layers are broken".
//!
//! Cards are placed *under* the text (see `Z_UNDER`), so a dialog is a normal
//! overlay: its opaque background hides the cards beneath it while the rest of
//! the canvas keeps showing and keeps updating.

use std::collections::{HashMap, VecDeque};
use std::io::{self, Write};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

use crate::shm::Shm;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Proto {
    Kitty,
    None,
}

pub struct Rgb {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

/// One card (or the zoomed picture in a modal) on screen.
pub struct Placement {
    pub pid: u32,
    pub path: String,
    /// cell position on screen
    pub x: u16,
    pub y: u16,
    /// size in cells
    pub c: u16,
    pub r: u16,
    /// source rectangle in picture pixels: (x, y, w, h)
    pub crop: Option<(u32, u32, u32, u32)>,
    /// stacking order; negative values put the picture under text and, below
    /// `i32::MIN / 2`, under cells that have a non-default background
    pub z: i32,
}

struct Uploaded {
    id: u32,
    w: u32,
    h: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Placed {
    img: u32,
    x: u16,
    y: u16,
    c: u16,
    r: u16,
    crop: Option<(u32, u32, u32, u32)>,
    z: i32,
}

pub struct Graphics {
    pub proto: Proto,
    ids: HashMap<String, Uploaded>,
    next_id: u32,
    placed: HashMap<u32, Placed>,
    shm: VecDeque<Shm>,
}

impl Graphics {
    pub fn new(proto: Proto) -> Self {
        Self {
            proto,
            ids: HashMap::new(),
            next_id: 1,
            placed: HashMap::new(),
            shm: VecDeque::new(),
        }
    }

    pub fn usable(&self) -> bool {
        self.proto == Proto::Kitty
    }

    /// Upload a picture if it is not known yet. Returns its pixel size.
    pub fn ensure<W: Write>(&mut self, out: &mut W, path: &str, rgb: &Rgb) -> io::Result<(u32, u32)> {
        if let Some(u) = self.ids.get(path) {
            return Ok((u.w, u.h));
        }
        if self.proto != Proto::Kitty {
            return Ok((rgb.w, rgb.h));
        }
        let id = self.next_id;
        self.next_id += 1;

        let mut shm = match Shm::new("ymi", rgb.data.len()) {
            Ok(s) => s,
            Err(_) => return Ok((rgb.w, rgb.h)),
        };
        shm.write(&rgb.data);
        let name = STANDARD.encode(shm.name().as_bytes());
        write!(
            out,
            "\x1b_Ga=t,f=24,s={},v={},i={},q=2,t=s,S={};{}\x1b\\",
            rgb.w,
            rgb.h,
            id,
            rgb.data.len(),
            name
        )?;
        self.shm.push_back(shm);
        while self.shm.len() > 8 {
            self.shm.pop_front();
        }
        self.ids.insert(
            path.to_string(),
            Uploaded {
                id,
                w: rgb.w,
                h: rgb.h,
            },
        );
        Ok((rgb.w, rgb.h))
    }

    /// Make the screen show exactly `want`. Tiles that are no longer wanted,
    /// moved, or now show a different picture are removed first.
    pub fn sync<W: Write>(&mut self, out: &mut W, want: &[Placement]) -> io::Result<()> {
        if self.proto != Proto::Kitty {
            return Ok(());
        }
        let mut desired: HashMap<u32, (Placed, &str)> = HashMap::with_capacity(want.len());
        for p in want {
            let Some(u) = self.ids.get(p.path.as_str()) else { continue };
            desired.insert(
                p.pid,
                (
                    Placed {
                        img: u.id,
                        x: p.x,
                        y: p.y,
                        c: p.c,
                        r: p.r,
                        crop: p.crop,
                        z: p.z,
                    },
                    p.path.as_str(),
                ),
            );
        }

        // Remove placements that are gone, moved or re-labelled. A placement is
        // the pair (image id, placement id), so both have to be named.
        let stale: Vec<u32> = self
            .placed
            .iter()
            .filter(|(pid, p)| match desired.get(pid) {
                None => true,
                Some((d, _)) => d != *p,
            })
            .map(|(pid, _)| *pid)
            .collect();
        for pid in &stale {
            if let Some(old) = self.placed.get(pid) {
                write!(out, "\x1b_Ga=d,d=i,i={},p={}\x1b\\", old.img, pid)?;
            }
            self.placed.remove(pid);
        }

        for (pid, (p, _)) in desired.iter() {
            if self.placed.get(pid) == Some(p) {
                continue;
            }
            write!(
                out,
                "\x1b[{};{}H\x1b_Ga=p,i={},p={},c={},r={},C=1,q=2",
                p.y + 1,
                p.x + 1,
                p.img,
                pid,
                p.c,
                p.r
            )?;
            if let Some((sx, sy, sw, sh)) = p.crop {
                write!(out, ",x={sx},y={sy},w={sw},h={sh}")?;
            }
            write!(out, ",z={}", p.z)?;
            write!(out, "\x1b\\")?;
            self.placed.insert(*pid, *p);
        }
        out.flush()?;
        Ok(())
    }

    /// Drop every image and placement (used on tab switches and on exit).
    pub fn clear<W: Write>(&mut self, out: &mut W) -> io::Result<()> {
        if self.proto == Proto::Kitty {
            write!(out, "\x1b_Ga=d,d=A\x1b\\")?;
            out.flush()?;
        }
        self.placed.clear();
        self.ids.clear();
        self.shm.clear();
        Ok(())
    }
}
