//! Command line: one table of options, rendered in the chosen language.
//!
//! `-h` prints one line per option, `--help` prints the full text; `-v` prints
//! the version and `--version` the build details. Both pairs come from the same
//! table, and clap's own help is switched off so the layout, wording and
//! language are ours.

use clap::Parser;

use crate::i18n::{self, t, Lang};

pub const NAME: &str = "ymi";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser, Debug)]
#[command(name = "ymi", disable_help_flag = true, disable_version_flag = true)]
pub struct Args {
    #[arg(default_value = "")]
    pub target: String,

    #[arg(long)]
    pub quit: bool,

    #[arg(long, hide = true)]
    pub daemon: bool,

    #[arg(long)]
    pub probe: bool,


    #[arg(long, hide = true)]
    pub render: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            target: String::new(),
            quit: false,
            daemon: false,
            probe: false,
            render: false,
        }
    }
}

struct Item {
    group: u8,
    flags: &'static str,
    flags_cn: Option<&'static str>,
    cn: (&'static str, &'static str),
    en: (&'static str, &'static str),
}

const GROUPS: [(&str, &str); 4] = [
    ("目标", "TARGET"),
    ("会话", "SESSION"),
    ("诊断", "DIAGNOSTICS"),
    ("其它", "MISC"),
];

const ITEMS: &[Item] = &[
    Item {
        group: 0,
        flags: "[target]",
        flags_cn: Some("[图片或目录]"),
        cn: (
            "图片或目录（默认空画布）",
            "给目录：画布 + 可搜索的图片列表，按 a 选图。\n\
             给图片：这张图铺满所有卡片。\n\
             什么都不给：一块只有 + 卡片的空画布。",
        ),
        en: (
            "picture or folder (default: empty canvas)",
            "A folder gives you the canvas plus a searchable picker on `a`.\n\
             A picture fills every card with it.\n\
             Nothing gives you an empty canvas of + cards.",
        ),
    },
    Item {
        group: 1,
        flags: "    --quit",
        flags_cn: None,
        cn: (
            "结束后台会话",
            "按 Esc 退出只是关掉界面，画布还留在后台；这个参数把后台也停掉。\n\
             Ctrl+Q 在界面里做同一件事。",
        ),
        en: (
            "stop the background session",
            "Esc only closes the UI; the canvas stays alive in the background. This\n\
             stops it for real. Ctrl+Q does the same from inside.",
        ),
    },
    Item {
        group: 2,
        flags: "    --probe",
        flags_cn: None,
        cn: (
            "打印终端能力后退出",
            "列出 TERM、Kitty 图形协议、单元格像素尺寸、键盘协议。\n\
             排查「图不显示」先跑这个。",
        ),
        en: (
            "print terminal capabilities and exit",
            "Shows TERM, kitty graphics support, cell size and the keyboard protocol.\n\
             First thing to run when pictures do not show up.",
        ),
    },
    Item {
        group: 3,
        flags: "    --lang <LANG>",
        flags_cn: None,
                cn: (
            "界面语言；单独使用只写配置、不启动界面",
            "影响 --help、--version、界面提示和报错。\n\
             单独执行 --lang en 只写入配置并退出，不启动界面。\n\
             配置保存在 ~/.config/ymi/config（$XDG_CONFIG_HOME 优先）。\n\
             以下写法等价：--lang en、--lang=en、--language en。\n\
             值不合法会直接报错。\n\
             默认：cn",
        ),
        en: (
            "UI language; alone it only saves the preference",
            "Applies to --help, --version, the interface and error messages.\n\
             On its own, --lang en only saves the preference and exits.\n\
             The file is ~/.config/ymi/config (XDG_CONFIG_HOME wins).\n\
             Equivalent: --lang en, --lang=en, --language en.\n\
             An unknown value is an error.\n\
             Default: cn",
        ),
    },
    Item {
        group: 3,
        flags: "-h",
        flags_cn: None,
        cn: (
            "这份简洁说明",
            "每个功能一句话，适合查参数名。\n要看取值、默认值和示例用 --help。",
        ),
        en: (
            "this summary",
            "One line per option: good for remembering a flag name.\n\
             Values, defaults and examples live in --help.",
        ),
    },
    Item {
        group: 3,
        flags: "    --help",
        flags_cn: None,
        cn: ("完整说明：取值、默认值、示例、排查", "即本页内容。"),
        en: ("full text: values, defaults, examples", "What you are reading."),
    },
    Item {
        group: 3,
        flags: "-v, --version",
        flags_cn: None,
        cn: (
            "版本（--version 给完整信息）",
            "-v 打一行版本号（`-V` 同义）。\n\
             --version 打完整信息：构建目标、终端要求、后台会话位置。",
        ),
        en: (
            "version (--version prints the full block)",
            "-v prints one line (`-V` works too).\n\
             --version prints build target, terminal requirements and where the\n\
             background session lives.",
        ),
    },
];

fn tagline() -> &'static str {
    t(
        "终端图片画布：图片按 1:1 卡片排布，可平移与缩放",
        "a pannable canvas of pictures in the terminal",
    )
}
/// Parse the command line.
///
/// The language flag is resolved first and then removed from the argument list,
/// so clap never sees it. Leaving it in is what let `--lang en` be read as
/// "`--lang` plus the positional `en`", which opened `./en` instead of switching
/// language.
pub fn parse() -> Args {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let (rest, wanted) = take_lang(&argv);

    // Explicit flag beats the saved preference, which beats Chinese.
    let lang = wanted
        .or_else(crate::config::read_lang)
        .unwrap_or(Lang::Cn);
    i18n::set(lang);

    // `-h` and `--version` use the language but must not change it.
    if let Some(code) = exits(&rest) {
        std::process::exit(code);
    }

    if let Some(l) = wanted {
        match crate::config::write_lang(l) {
            Ok(path) => {
                // Asking only for a language means "configure", not "start".
                if rest.is_empty() {
                    println!(
                        "{}",
                        i18n::p(
                            format!("语言已设为 {}，写入 {}", l.code(), path.display()),
                            format!("language set to {} in {}", l.code(), path.display()),
                        )
                    );
                    std::process::exit(0);
                }
            }
            Err(e) => {
                eprintln!(
                    "{}",
                    i18n::p(
                        format!("{NAME}: 无法写入配置文件：{e}"),
                        format!("{NAME}: cannot write the config file: {e}"),
                    )
                );
                if rest.is_empty() {
                    std::process::exit(1);
                }
            }
        }
    }
    match Args::try_parse_from(std::iter::once(NAME.to_string()).chain(rest)) {
        Ok(a) => a,
        Err(e) => {
            report(e);
            std::process::exit(2);
        }
    }
}

/// Pull `--lang` / `--language` out of the arguments and settle the language.
fn take_lang(argv: &[String]) -> (Vec<String>, Option<Lang>) {
    let mut rest = Vec::with_capacity(argv.len());
    let mut lang: Option<Lang> = None;
    let mut bad: Option<String> = None;
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        if let Some(v) = a
            .strip_prefix("--lang=")
            .or_else(|| a.strip_prefix("--language="))
        {
            match Lang::parse(v) {
                Some(l) => lang = Some(l),
                None => bad = Some(v.to_string()),
            }
            i += 1;
        } else if a == "--lang" || a == "--language" {
            match argv.get(i + 1) {
                Some(next) if !next.starts_with('-') => {
                    match Lang::parse(next) {
                        Some(l) => lang = Some(l),
                        None => bad = Some(next.clone()),
                    }
                    i += 2;
                }
                // A bare `--lang` is a mistake worth naming, not a silent cn.
                _ => {
                    bad = Some(String::new());
                    i += 1;
                }
            }
        } else {
            rest.push(a.clone());
            i += 1;
        }
    }

    if let Some(v) = bad {
        // The language is exactly what could not be settled, so say it twice.
        if v.is_empty() {
            eprintln!("{NAME}: --lang 需要一个值，取 cn 或 en / --lang needs a value: cn or en");
        } else {
            eprintln!(
                "{NAME}: 无法识别的语言 \"{v}\"，取 cn 或 en / unknown language \"{v}\", use cn or en"
            );
        }
        std::process::exit(2);
    }
    (rest, lang)
}

/// `-h`, `--help`, `-v`, `--version` and `--lang` are handled before clap so
/// the language is settled first and help can be printed in it.
fn exits(argv: &[String]) -> Option<i32> {
    for (i, a) in argv.iter().enumerate() {
        // `-hv` / `-vh`: a cluster made only of help/version letters means help.
        let cluster = a.len() > 2
            && a.starts_with('-')
            && !a.starts_with("--")
            && a[1..].chars().all(|c| matches!(c, 'h' | 'v' | 'V'));
        match a.as_str() {
            "-h" => {
                short_help();
                return Some(0);
            }
            "--help" => {
                long_help();
                return Some(0);
            }
            "-v" | "-V" => {
                // `-v 80` used to mean volume; say why it printed a version
                // instead of silently swallowing the number.
                if let Some(next) = argv.get(i + 1) {
                    if !next.is_empty() && next.chars().all(|c| c.is_ascii_digit()) {
                        eprintln!(
                            "{}",
                            t(
                                "ymi 提示：音量已改为 --volume，-v 现在只打印版本号",
                                "ymi: volume is --volume now; -v only prints the version"
                            )
                        );
                    }
                }
                version_line();
                return Some(0);
            }
            "--version" => {
                long_version();
                return Some(0);
            }
            _ if cluster => {
                if a.contains('h') {
                    short_help();
                } else {
                    version_line();
                }
                return Some(0);
            }
            _ => {}
        }
    }
    None
}

fn flag_of(it: &Item, en: bool) -> &'static str {
    if en {
        it.flags
    } else {
        it.flags_cn.unwrap_or(it.flags)
    }
}

fn pad(s: &str, width: usize) -> String {
    let w = i18n::width(s);
    if w >= width {
        format!("{s} ")
    } else {
        format!("{s}{}", " ".repeat(width - w))
    }
}

fn version_line() {
    println!("{NAME} {VERSION}");
}

fn short_help() {
    let en = i18n::is_en();
    println!("{NAME} {VERSION} · {}", tagline());
    println!();
    println!("{}", t("用法", "Usage"));
    println!(
        "  {NAME} {} [{}]",
        t("[选项]", "[options]"),
        t("图片或目录", "picture or folder")
    );
    println!();
    println!("{}", t("选项", "Options"));
    for it in ITEMS {
        let line = if en { it.en.0 } else { it.cn.0 };
        println!("  {}{}", pad(flag_of(it, en), 22), line);
    }
    println!();
    println!(
        "{}",
        t(
            "取值、默认值和示例：ymi --help　　界面内按 ? 看快捷键",
            "values, defaults and examples: ymi --help    press ? for the key map"
        )
    );
}

fn long_help() {
    let en = i18n::is_en();
    println!("{NAME} {VERSION} · {}", tagline());
    println!();
    println!("{}", t("用法", "Usage"));
    println!(
        "  {NAME} {} [{}]",
        t("[选项]", "[options]"),
        t("图片或目录", "picture or folder")
    );
    println!();

    let mut group = u8::MAX;
    for it in ITEMS {
        if it.group != group {
            group = it.group;
            let (cn, en_t) = GROUPS[group as usize];
            println!("{}", if en { en_t } else { cn });
        }
        println!("  {}", flag_of(it, en));
        let text = if en { it.en.1 } else { it.cn.1 };
        for line in text.lines() {
            println!("      {line}");
        }
        println!();
    }

    println!("{}", t("示例", "Examples"));
    if en {
        println!("  ymi                        open the canvas");
        println!("  ymi ~/Pictures             canvas plus a picker over that folder");
        println!("  ymi ~/Pictures/a.jpg       fill the cards with this picture");
        println!("  ymi --quit                 stop the background session");
        println!("  ymi --probe                check terminal capabilities");
    } else {
        println!("  ymi                        打开画布");
        println!("  ymi ~/Pictures             画布 + 这个目录的图片列表");
        println!("  ymi ~/Pictures/a.jpg       把这张图铺上画布");
        println!("  ymi --quit                 结束后台会话");
        println!("  ymi --probe                查看终端能力");
    }
    println!();
    println!("{}", t("说明", "Notes"));
    if en {
        println!("  Esc leaves the UI but keeps the canvas alive, so reopening is instant;");
        println!("  closing the terminal window ends it too. Ctrl+Q stops it from inside.");
    } else {
        println!("  Esc 只关界面，画布留在后台，下次打开还在原处；");
        println!("  关掉终端窗口也会顺带结束它。界面里 Ctrl+Q 是连后台一起退。");
    }
    println!();
    println!("{}", t("退出码", "Exit codes"));
    if en {
        println!("  0 ok   1 runtime error   2 bad arguments");
    } else {
        println!("  0 正常   1 运行出错   2 参数有误");
    }
}

fn long_version() {
    let target = format!(
        "{}-{}",
        std::env::consts::ARCH,
        match std::env::consts::OS {
            "macos" => "apple-darwin",
            "linux" => "unknown-linux-gnu",
            other => other,
        }
    );
    println!("{NAME} {VERSION}");
    println!("{}", tagline());
    println!();
    if i18n::is_en() {
        println!("build target   {target}");
        println!("terminal       kitty graphics protocol, frames over shared memory");
        println!("session        one background daemon per socket");
        println!("               (YMI_SOCK overrides the default socket)");
        println!("licence        MIT");
        println!("note           AI-generated, macOS only, no issue/PR support");
    } else {
        println!("构建目标    {target}");
        println!("终端要求    Kitty 图形协议，帧走共享内存");
        println!("后台会话    每个 socket 一个后台进程（YMI_SOCK 可以指定）");
        println!("许可        MIT");
        println!("说明        由 AI 生成，仅适配 macOS，不提供 issue / PR 支持");
    }
}

fn report(e: clap::Error) {
    use clap::error::{ContextKind, ContextValue, ErrorKind};

    let what = match e.kind() {
        ErrorKind::UnknownArgument => t("未知参数", "unknown argument"),
        ErrorKind::InvalidValue | ErrorKind::ValueValidation => t("参数值不合法", "invalid value"),
        ErrorKind::MissingRequiredArgument => t("缺少必需的参数", "missing argument"),
        ErrorKind::TooManyValues => t("给了太多值", "too many values"),
        ErrorKind::NoEquals => t("这里需要等号", "expected '='"),
        _ => t("参数有误", "bad arguments"),
    };
    let arg = match e.get(ContextKind::InvalidArg) {
        Some(ContextValue::String(s)) => format!(" `{s}`"),
        _ => String::new(),
    };
    let bad = match e.get(ContextKind::InvalidValue) {
        Some(ContextValue::String(s)) => format!(" = {s}"),
        _ => String::new(),
    };
    // Listing the accepted values is the difference between "it does not work"
    // and "oh, shuffle is spelt with one f".
    let mut valid: Vec<String> = Vec::new();
    for (kind, value) in e.context() {
        if kind == ContextKind::ValidValue {
            if let ContextValue::String(s) = value {
                valid.push(s.clone());
            }
        }
    }
    if valid.is_empty() {
        // clap renders the accepted values into the message instead of the
        // context; lift them out so they can be shown in either language.
        if let Some(rest) = e.to_string().split("possible values: ").nth(1) {
            let line = rest.lines().next().unwrap_or("");
            let line = line.trim().trim_start_matches('[').trim_end_matches(']');
            valid = line
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }

    eprintln!("{NAME}: {what}{arg}{bad}");
    if !valid.is_empty() {
        eprintln!("       {} {}", t("可选：", "valid:"), valid.join(" / "));
    }
    eprintln!(
        "       {}",
        t(
            "用 -h 看简洁用法，--help 看完整说明",
            "run `-h` for a summary, `--help` for the full text"
        )
    );
}
