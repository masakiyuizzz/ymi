use std::io::prelude::*;
use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Caps {
    pub kitty: bool,
    pub iterm: bool,
    pub light: bool,
    pub cell_w: u16,
    pub cell_h: u16,
    pub cell_detected: bool,
    /// terminal answers the kitty keyboard protocol
    pub kbd: bool,
    pub term: String,
    pub term_program: String,
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            kitty: false,
            iterm: false,
            light: false,
            cell_w: 8,
            cell_h: 17,
            cell_detected: false,
            kbd: false,
            term: std::env::var("TERM").unwrap_or_default(),
            term_program: std::env::var("TERM_PROGRAM").unwrap_or_default(),
        }
    }
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false)
}

fn env_contains(name: &str, needle: &str) -> bool {
    std::env::var(name)
        .map(|v| v.to_lowercase().contains(needle))
        .unwrap_or(false)
}

/// Ask the terminal who it is. We talk straight to /dev/tty so that crossterm's
/// parser (which would silently eat APC replies) never sees the answers.
pub fn probe() -> Caps {
    let mut caps = Caps::default();

    // Cheap heuristics first.
    let t = caps.term.to_lowercase();
    let tp = caps.term_program.to_lowercase();
    if t.contains("kitty")
        || t.contains("ghostty")
        || t.contains("wezterm")
        || tp.contains("ghostty")
        || tp.contains("wezterm")
        || tp.contains("kitty")
        || env_truthy("KITTY_WINDOW_ID")
        || env_truthy("WEZTERM_PANE")
        || env_truthy("GHOSTTY_RESOURCES_DIR")
    {
        caps.kitty = true;
    }
    if tp.contains("iterm") || env_contains("LC_TERMINAL", "iterm") {
        caps.iterm = true;
    }
    if t == "dumb" || t.is_empty() {
        eprintln!("ymi: TERM is not set to a usable value (TERM={:?})", caps.term);
    }

    let stdin_tty = unsafe { libc::isatty(0) } == 1;
    let stdout_tty = unsafe { libc::isatty(1) } == 1;
    if !stdin_tty || !stdout_tty {
        return caps;
    }

    let Ok(mut tty) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
    else {
        return caps;
    };

    // Replies are only readable byte-by-byte in raw mode (canonical mode would
    // wait for a newline that never comes).
    let _ = crossterm::terminal::enable_raw_mode();

    let mut query = String::new();
    // Kitty graphics support query (no-op if unsupported).
    query.push_str("\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\");
    // Background colour.
    query.push_str("\x1b]11;?\x07");
    // Kitty keyboard protocol (Ctrl+digit and friends need CSI-u).
    query.push_str("\x1b[?u");
    // Cell size in pixels, then window px + window cells as a fallback.
    query.push_str("\x1b[16t");
    query.push_str("\x1b[14t");
    query.push_str("\x1b[18t");
    if tty.write_all(query.as_bytes()).is_err() || tty.flush().is_err() {
        return caps;
    }

    let fd = tty.as_raw_fd();

    // Non blocking: a blocking read() here would swallow the user's first
    // keystroke while we wait for a reply that may never come.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags >= 0 {
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }

    let mut buf: Vec<u8> = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(240);
    let mut chunk = [0u8; 1024];
    while Instant::now() < deadline {
        let mut fds = [libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        }];
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), 1, 25) };
        if ready <= 0 {
            // Nothing (more) to read; stop early once the two answers we care
            // about are in, otherwise keep waiting out the budget.
            if parse_kitty(&buf).is_some() && parse_bg(&buf).is_some() {
                break;
            }
            continue;
        }
        match tty.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > 8192 {
                    break;
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }

    caps.kbd = parse_kbd(&buf);

    let _ = crossterm::terminal::disable_raw_mode();

    if let Some(kitty) = parse_kitty(&buf) {
        caps.kitty = kitty;
    }
    if let Some(light) = parse_bg(&buf) {
        caps.light = light;
    }
    if let Some((w, h)) = parse_cell(&buf) {
        caps.cell_w = w;
        caps.cell_h = h;
        caps.cell_detected = true;
    } else if let Some((w, h)) = parse_window(&buf) {
        caps.cell_w = w;
        caps.cell_h = h;
        caps.cell_detected = true;
    }

    // Sanity clamp: some terminals report nonsense.
    if !(3..=64).contains(&caps.cell_w) || !(6..=96).contains(&caps.cell_h) {
        caps.cell_w = 8;
        caps.cell_h = 17;
        caps.cell_detected = false;
    }
    caps
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn parse_kitty(buf: &[u8]) -> Option<bool> {
    let idx = find_subslice(buf, b"\x1b_G")?;
    let tail = &buf[idx..];
    let end = tail
        .windows(2)
        .position(|w| w == b"\x1b\\")
        .unwrap_or(tail.len());
    let body = String::from_utf8_lossy(&tail[..end]);
    Some(body.contains("OK"))
}

/// Reply to `CSI ? u`: `CSI ? <flags> u` means the kitty keyboard protocol is
/// understood; no reply means Ctrl+digit cannot be told apart from plain keys.
fn parse_kbd(buf: &[u8]) -> bool {
    let text = String::from_utf8_lossy(buf);
    let Some(rest) = text.split("\x1b[?").nth(1) else {
        return false;
    };
    let mut digits = String::new();
    for ch in rest.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else {
            return ch == 'u' && !digits.is_empty();
        }
    }
    false
}

fn parse_bg(buf: &[u8]) -> Option<bool> {
    let idx = find_subslice(buf, b"\x1b]11;rgb:")?;
    let tail = String::from_utf8_lossy(&buf[idx + 9..]);
    let tail = tail.split(|c| c == '\x07' || c == '\x1b').next()?;
    let mut parts = tail.split('/');
    let r = u16::from_str_radix(parts.next()?.trim(), 16).ok()?;
    let g = u16::from_str_radix(parts.next()?.trim(), 16).ok()?;
    let b = u16::from_str_radix(parts.next()?.trim(), 16).ok()?;
    let lum = 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32;
    Some(lum > 0.5 * 65535.0)
}

fn parse_cell(buf: &[u8]) -> Option<(u16, u16)> {
    let text = String::from_utf8_lossy(buf);
    let idx = text.find("\x1b[6;")?;
    let rest = &text[idx + 4..];
    let body = rest.split('t').next()?;
    let mut it = body.split(';');
    let h: u16 = it.next()?.trim().parse().ok()?;
    let w: u16 = it.next()?.trim().parse().ok()?;
    if w == 0 || h == 0 {
        None
    } else {
        Some((w, h))
    }
}

fn parse_window(buf: &[u8]) -> Option<(u16, u16)> {
    let text = String::from_utf8_lossy(buf);
    let px_idx = text.find("\x1b[4;")?;
    let px = text[px_idx + 4..].split('t').next()?;
    let mut pxi = px.split(';');
    let ph: u32 = pxi.next()?.trim().parse().ok()?;
    let pw: u32 = pxi.next()?.trim().parse().ok()?;

    let ch_idx = text.find("\x1b[8;")?;
    let ch = text[ch_idx + 4..].split('t').next()?;
    let mut chi = ch.split(';');
    let chh: u32 = chi.next()?.trim().parse().ok()?;
    let chw: u32 = chi.next()?.trim().parse().ok()?;

    if chw == 0 || chh == 0 {
        return None;
    }
    let w = (pw / chw).max(1) as u16;
    let h = (ph / chh).max(1) as u16;
    Some((w, h))
}

static KBD_PUSHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn enter(kbd: bool) -> std::io::Result<()> {
    crossterm::terminal::enable_raw_mode()?;
    let mut out = std::io::stdout();
    crossterm::execute!(
        out,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture,
        crossterm::cursor::Hide,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
    )?;
    // Ask the terminal for unambiguous modified keys (Ctrl+1, Ctrl+=, ...).
    // Terminals without the kitty keyboard protocol ignore this.
    if kbd {
        let pushed = crossterm::execute!(
            out,
            crossterm::event::PushKeyboardEnhancementFlags(
                crossterm::event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            )
        )
        .is_ok();
        KBD_PUSHED.store(pushed, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(())
}

pub fn leave() -> std::io::Result<()> {
    let mut out = std::io::stdout();
    if KBD_PUSHED.swap(false, std::sync::atomic::Ordering::Relaxed) {
        let _ = crossterm::execute!(
            out,
            crossterm::event::PopKeyboardEnhancementFlags
        );
    }
    crossterm::execute!(
        out,
        crossterm::event::DisableMouseCapture,
        crossterm::cursor::Show,
        crossterm::terminal::LeaveAlternateScreen
    )?;
    crossterm::terminal::disable_raw_mode()?;
    Ok(())
}

pub fn print_caps(caps: &Caps) {
    let en = crate::i18n::is_en();
    let label = |cn: &str, e: &str| -> String {
        let text = if en { e } else { cn };
        let pad = 18usize.saturating_sub(crate::i18n::width(text));
        format!("{text}{}", " ".repeat(pad))
    };
    let yes = crate::i18n::t("是", "yes");
    let no = crate::i18n::t("否", "no");

    println!(
        "{}",
        crate::i18n::p(
            format!("{} 终端探测", crate::cli::NAME),
            format!("{} terminal probe", crate::cli::NAME)
        )
    );
    println!("  {} {}", label("TERM", "TERM"), caps.term);
    println!("  {} {}", label("TERM_PROGRAM", "TERM_PROGRAM"), caps.term_program);
    println!(
        "  {} {}",
        label("kitty 键盘协议", "kitty keyboard"),
        if caps.kbd { yes } else { no }
    );
    println!(
        "  {} {}",
        label("kitty 图形协议", "kitty graphics"),
        if caps.kitty { yes } else { no }
    );
    println!(
        "  {} {}",
        label("iterm2 内联图", "iterm2 inline"),
        if caps.iterm { yes } else { no }
    );
    println!(
        "  {} {}",
        label("背景", "background"),
        if caps.light {
            crate::i18n::t("浅色", "light")
        } else {
            crate::i18n::t("深色", "dark")
        }
    );
    println!(
        "  {} {}x{} px{}",
        label("单元格尺寸", "cell size"),
        caps.cell_w,
        caps.cell_h,
        if caps.cell_detected {
            ""
        } else {
            crate::i18n::t("（回退估算）", " (fallback guess)")
        }
    );
    if !caps.kitty {
        println!();
        if en {
            println!("note: the picture needs the kitty graphics protocol, which this terminal did");
            println!("      not answer for. There is no fallback — try kitty, WezTerm or ELYTerminal.");
        } else {
            println!("提示：画面要用 Kitty 图形协议，这条终端没有回应这个探测。");
            println!("      没有替代方案，换一个支持的终端：kitty / WezTerm / ELYTerminal。");
        }
    }
}
