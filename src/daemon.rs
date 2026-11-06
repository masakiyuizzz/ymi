//! The background session.
//!
//! `ymi` starts a small daemon the first time it runs. The daemon owns the
//! session (tabs, positions, picture lists) so that closing the UI does not
//! lose anything, and it holds the original terminal open: the moment that
//! terminal goes away the daemon sees POLLHUP and exits, taking the session
//! with it.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::canvas::Session;

#[derive(Serialize, Deserialize)]
#[serde(tag = "op")]
enum Msg {
    Hello,
    State { session: Session },
    Quit,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "op")]
enum Reply {
    State { session: Session },
    Ok,
}

pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("YMI_SOCK") {
        return PathBuf::from(p);
    }
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("ymi.sock");
    }
    let tmp = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    tmp.join(format!("ymi-{}.sock", unsafe { libc::getuid() }))
}

pub struct Client {
    stream: UnixStream,
}

impl Client {
    /// Attach to the running session, starting the daemon if there is none.
    pub fn attach() -> Result<(Client, Session)> {
        let path = socket_path();
        if let Some(hit) = Client::try_connect(&path) {
            return Ok(hit);
        }
        let _ = std::fs::remove_file(&path);
        spawn_daemon(&path)?;

        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if let Some(hit) = Client::try_connect(&path) {
                return Ok(hit);
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        Err(anyhow!("background session did not start"))
    }

    fn try_connect(path: &Path) -> Option<(Client, Session)> {
        let stream = UnixStream::connect(path).ok()?;
        let mut c = Client { stream };
        c.send(&Msg::Hello).ok()?;
        match c.recv() {
            Ok(Reply::State { session }) => Some((c, session)),
            _ => None,
        }
    }

    pub fn save(&mut self, session: &Session) -> Result<()> {
        self.send(&Msg::State {
            session: session.clone(),
        })?;
        let _ = self.recv();
        Ok(())
    }

    pub fn quit(&mut self) -> Result<()> {
        self.send(&Msg::Quit)
    }

    fn send(&mut self, msg: &Msg) -> Result<()> {
        let mut line = serde_json::to_vec(msg)?;
        line.push(b'\n');
        self.stream.write_all(&line)?;
        self.stream.flush()?;
        Ok(())
    }

    fn recv(&mut self) -> Result<Reply> {
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            let n = self.stream.read(&mut byte)?;
            if n == 0 {
                return Err(anyhow!("session closed"));
            }
            if byte[0] == b'\n' {
                break;
            }
            line.push(byte[0]);
            if line.len() > 8 << 20 {
                return Err(anyhow!("oversized reply"));
            }
        }
        Ok(serde_json::from_slice(&line)?)
    }
}

fn spawn_daemon(path: &Path) -> Result<()> {
    let exe = std::env::current_exe().context("cannot find my own binary")?;
    let tty = std::env::var("YMI_TTY").unwrap_or_default();
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--daemon")
        .env("YMI_SOCK", path)
        .env("YMI_TTY", tty)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(if std::env::var_os("YMI_DEBUG").is_some() {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/ymi-daemon.log")
                .map(std::process::Stdio::from)
                .unwrap_or_else(|_| std::process::Stdio::null())
        } else {
            std::process::Stdio::null()
        });
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            // Own session: the shell going away must not take the session with
            // it. The terminal itself is watched through a separate fd.
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn()?;
    Ok(())
}

/// Run the background session. Called as `ymi --daemon`.
pub fn run(socket: &Path, tty: Option<String>) -> Result<()> {
    unsafe {
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }
    log(&format!(
        "daemon start pid={} sock={} tty={:?}",
        std::process::id(),
        socket.display(),
        tty
    ));
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket).with_context(|| format!("bind {}", socket.display()))?;
    listener.set_nonblocking(true)?;

    let session = Arc::new(Mutex::new(Session::default()));
    let mut tty_fd: Option<(i32, std::fs::File)> = None;
    if let Some(p) = tty {
        if let Ok(f) = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOCTTY)
            .open(&p)
        {
            let fd = std::os::unix::io::AsRawFd::as_raw_fd(&f);
            tty_fd = Some((fd, f));
        }
    }

    loop {
        if let Some((fd, _keep)) = &tty_fd {
            let mut p = libc::pollfd {
                fd: *fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let r = unsafe { libc::poll(&mut p, 1, 0) };
            if r > 0 && (p.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL)) != 0 {
                log(&format!("daemon: tty hup (revents={})", p.revents));
                let _ = std::fs::remove_file(socket);
                return Ok(());
            }
        }

        match listener.accept() {
            Ok((stream, _)) => {
                // A socket accepted from a non-blocking listener inherits that
                // flag on macOS, which makes the reader see EAGAIN and hang up
                // after the first message.
                let _ = stream.set_nonblocking(false);
                let s = session.clone();
                std::thread::spawn(move || {
                    let _ = serve(stream, s);
                });
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.into()),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn serve(stream: UnixStream, session: Arc<Mutex<Session>>) -> Result<()> {
    let mut writer = stream.try_clone()?;
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let msg: Msg = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(_) => continue,
        };
        match msg {
            Msg::Hello => {
                let s = session.lock().unwrap().clone();
                reply(&mut writer, &Reply::State { session: s })?;
            }
            Msg::State { session: s } => {
                *session.lock().unwrap() = s;
                reply(&mut writer, &Reply::Ok)?;
            }
            Msg::Quit => {
                let _ = std::fs::remove_file(socket_path());
                std::process::exit(0);
            }
        }
    }
    Ok(())
}

fn log(msg: &str) {
    if std::env::var_os("YMI_DEBUG").is_some() {
        eprintln!("{msg}");
    }
}

fn reply(w: &mut UnixStream, r: &Reply) -> Result<()> {
    let mut line = serde_json::to_vec(r)?;
    line.push(b'\n');
    w.write_all(&line)?;
    w.flush()?;
    Ok(())
}
