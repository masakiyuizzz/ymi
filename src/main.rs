mod app;
mod canvas;
mod cli;
mod config;
mod daemon;
mod graphics;
mod i18n;
mod scan;
mod shm;
mod term;
mod theme;
mod ui;

use anyhow::Result;
fn main() {
    let args = cli::parse();

    if args.daemon {
        let tty = std::env::var("YMI_TTY").ok();
        if let Err(e) = daemon::run(&daemon::socket_path(), tty) {
            eprintln!("ymi daemon: {e}");
            std::process::exit(1);
        }
        return;
    }

    if args.quit {
        match daemon::Client::attach() {
            Ok((mut c, _)) => {
                let _ = c.quit();
                println!(
                    "{}",
                    i18n::t("后台会话已结束", "background session stopped")
                );
            }
            Err(_) => println!(
                "{}",
                i18n::t("没有运行中的会话", "no session running")
            ),
        }
        return;
    }

    if let Err(e) = run(args) {
        eprintln!("ymi: {e}");
        std::process::exit(1);
    }
}

fn tty_name(fd: i32) -> Option<String> {
    unsafe {
        let p = libc::ttyname(fd);
        if p.is_null() {
            return None;
        }
        Some(std::ffi::CStr::from_ptr(p).to_string_lossy().to_string())
    }
}

fn run(args: cli::Args) -> Result<()> {
    // Remember which terminal we came from: the background session watches it
    // and exits when the window goes away.
    if std::env::var_os("YMI_TTY").is_none() {
        if let Some(name) = tty_name(0).or_else(|| tty_name(1)) {
            std::env::set_var("YMI_TTY", name);
        }
    }

    let caps = term::probe();
    if args.probe {
        term::print_caps(&caps);
        return Ok(());
    }
    if !caps.kitty {
        eprintln!(
            "{}",
            i18n::p(
                format!(
                    "ymi: 终端没有回应 Kitty 图形协议，图片无法显示\n\
                     \x20     TERM={}\n\
                     \x20     画布上的每张图都是真图，靠 Kitty 图形协议画到终端里，没有替代方案。\n\
                     \x20     支持该协议的终端：kitty、WezTerm、ELYTerminal，换一个再试。\n\
                     \x20     排查：ymi --probe",
                    caps.term
                ),
                format!(
                    "ymi: this terminal did not answer the kitty graphics probe\n\
                     \x20     TERM={}\n\
                     \x20     every card is a real image drawn with the kitty graphics protocol;\n\
                     \x20     there is no fallback. Try kitty, WezTerm or ELYTerminal.\n\
                     \x20     diagnose: ymi --probe",
                    caps.term
                )
            )
        );
        std::process::exit(1);
    }

    let (client, session) = daemon::Client::attach()?;
    let kbd = caps.kbd;
    let mut app = app::App::new(&args, caps, session, client)?;

    term::enter(kbd)?;
    let result = app.run();
    let _ = term::leave();
    result
}
