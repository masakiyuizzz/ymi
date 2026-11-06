use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, BorderType, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::i18n;
use crate::app::{App, Modal, Picker};
use crate::theme::Theme;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let t = app.theme.clone();
    f.render_widget(Block::default().style(Style::default().bg(Color::Reset)), area);

    if area.width < 30 || area.height < 8 {
        f.render_widget(
            Paragraph::new(i18n::t("窗口太小", "too small"))
                .alignment(Alignment::Center)
                .style(Style::default().fg(t.muted)),
            area,
        );
        return;
    }

    let frame = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.border));
    let inner = frame.inner(area);
    f.render_widget(frame, area);

    let top = Rect {
        x: area.x + 1,
        y: area.y,
        width: inner.width,
        height: 1,
    };
    f.render_widget(Clear, top);
    f.render_widget(Paragraph::new(top_line(app, inner.width as usize)), top);

    if let Some(r) = app.tabs_row {
        draw_tabs(f, app, r);
    }
    draw_tiles(f, app, &t);
    draw_status(f, app, app.status_row);

    match &app.modal {
        Some(Modal::Help) => draw_help(f, app, app.content),
        Some(Modal::Image(path)) => draw_image_modal(f, app, path.clone()),
        Some(Modal::Picker) => {
            if let Some(p) = &app.picker {
                draw_picker(f, app, p);
            }
        }
        Some(Modal::Input) => draw_input(f, app),
        None => {}
    }

    // A toast would land on top of the dialog's own frame; show it after the
    // dialog closes instead.
    if app.modal.is_none() {
        if let Some((msg, _)) = &app.toast {
            draw_toast(f, app, msg.clone());
        }
    }
}

pub fn modal_rect(content: Rect) -> Rect {
    let w = ((content.width as f32 * 0.9) as u16)
        .max(24)
        .min(content.width.saturating_sub(2));
    let h = ((content.height as f32 * 0.9) as u16)
        .max(8)
        .min(content.height.saturating_sub(2));
    Rect {
        x: content.x + (content.width.saturating_sub(w)) / 2,
        y: content.y + (content.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

pub fn modal_image_rect(content: Rect, _cw: u16, _ch: u16) -> Rect {
    let m = modal_rect(content);
    Rect {
        x: m.x + 1,
        y: m.y + 2,
        width: m.width.saturating_sub(2),
        height: m.height.saturating_sub(3),
    }
}

fn top_line(app: &App, width: usize) -> Line<'static> {
    let t = &app.theme;
    let canvas = app.session.tab();
    let place = canvas
        .root
        .as_deref()
        .map(short_path)
        .unwrap_or_else(|| i18n::t("无限画布", "endless canvas").to_string());
    let right = format!(
        "{} 张 · {}%{}",
        canvas.images.len(),
        (canvas.zoom * 100.0).round() as i32,
        if app.session.tabs.len() > 1 {
            format!(" · {}", app.session.tabs.len())
        } else {
            String::new()
        }
    );

    let brand = Span::styled(
        " ymi ",
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    );
    let room = width.saturating_sub(1 + 5 + 2 + 2 + right.len());
    let title = fit(&place, room);
    let used = 1 + 5 + 2 + UnicodeWidthStr::width(title.as_str()) + 2 + right.len();
    let fill = width.saturating_sub(used);
    Line::from(vec![
        Span::styled("─", Style::default().fg(t.border)),
        brand,
        Span::styled(format!(" {title} "), Style::default().fg(t.muted)),
        Span::styled("─".repeat(fill), Style::default().fg(t.faint)),
        Span::styled(format!(" {right} "), Style::default().fg(t.dim)),
    ])
}

fn draw_tabs(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let mut spans = Vec::new();
    for (i, tab) in app.session.tabs.iter().enumerate() {
        let label = format!(
            " {}{} ",
            i + 1,
            if tab.images.is_empty() { "" } else { "•" }
        );
        let style = if i == app.session.active {
            Style::default()
                .fg(Color::Rgb(0x16, 0x18, 0x1e))
                .bg(t.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.dim)
        };
        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_tiles(f: &mut Frame, app: &App, t: &Theme) {
    // Filled cards are just the pictures, edge to edge — the grid is one sheet.
    // Empty cards show a single "+" so you can see where to click.
    for view in &app.tiles {
        if view.image.is_some() {
            continue;
        }
        let rect = &view.rect;
        if rect.width < 1 || rect.height < 1 {
            continue;
        }
        let hovered = app.hover == Some((view.i, view.j));
        let style = if hovered {
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.dim)
        };
        let plus = Rect {
            x: rect.x + rect.width / 2,
            y: rect.y + rect.height / 2,
            width: 1,
            height: 1,
        };
        f.render_widget(Paragraph::new(Span::styled("+", style)), plus);
    }
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 || area.width < 10 {
        return;
    }
    let t = &app.theme;
    let canvas = app.session.tab();
    // The cards carry no border, so the name of what is under the pointer is
    // shown here instead of a highlight.
    let hovered = app
        .hover
        .and_then(|(i, j)| app.tiles.iter().find(|v| v.i == i && v.j == j))
        .and_then(|v| v.image.as_deref())
        .and_then(|p| std::path::Path::new(p).file_name())
        .map(|n| n.to_string_lossy().to_string());
    let hint = match &hovered {
        Some(name) => format!(" {name} "),
        None => i18n::t(
        " 拖拽平移 · 滚轮移动 · 点击查看 · a 选图 · x 删除 · ? 帮助",
        " drag to pan · wheel to move · click to open · a pick · x remove · ? help",
    ).to_string(),
    };
    let right = format!(
        "{} 张 · {}% · {:.0},{:.0} ",
        canvas.images.len(),
        (canvas.zoom * 100.0).round() as i32,
        canvas.ox,
        canvas.oy
    );
    let room = (area.width as usize).saturating_sub(right.len() + 2);
    let left = fit(&hint, room);
    let used = 1 + UnicodeWidthStr::width(left.as_str()) + right.len() + 1;
    let pad = (area.width as usize).saturating_sub(used);
    let style = if hovered.is_some() {
        Style::default().fg(t.muted)
    } else {
        Style::default().fg(t.dim)
    };
    let line = Line::from(vec![
        Span::styled(format!(" {left}"), style),
        Span::raw(" ".repeat(pad)),
        Span::styled(right, Style::default().fg(t.faint)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn draw_image_modal(f: &mut Frame, app: &App, path: String) {
    let t = &app.theme;
    let m = modal_rect(app.content);
    f.render_widget(Clear, m);
    let name = std::path::Path::new(&path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or(path.clone());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.accent))
        .title(Span::styled(
            format!(" {name} "),
            Style::default().fg(t.accent),
        ))
        .style(Style::default().bg(Color::Reset));
    f.render_widget(block, m);

    let hint = Rect {
        x: m.x + 1,
        y: m.y + 1,
        width: m.width.saturating_sub(2),
        height: 1,
    };
    f.render_widget(
        Paragraph::new(Span::styled(
            i18n::t(" Esc 关闭 · x 移除这张 ", " Esc closes · x removes this one "),
            Style::default().fg(t.dim),
        )),
        hint,
    );
}

fn draw_help(f: &mut Frame, app: &App, content: Rect) {
    let t = &app.theme;
    let keys: &[(&str, &str)] = if i18n::is_en() {
        &[
            ("drag", "pan in any direction, flick to glide"),
            ("wheel", "scroll up-down / left-right, eased"),
            ("Ctrl + =/-", "zoom in / out around the centre"),
            ("Ctrl + 0", "back to 100%"),
            ("click +", "pick a picture (a list when opened on a folder)"),
            ("a / +", "picker with a live preview on the right"),
            ("x / Delete", "remove the picture under the pointer"),
            ("hover", "the status bar names the picture"),
            ("click a picture", "open it full width"),
            ("Ctrl + T / W", "new / close a canvas"),
            ("Ctrl + 1~9", "switch canvas"),
            ("Esc", "leave the UI, keep the background session"),
            ("Ctrl + Q", "quit the background session too"),
        ]
    } else {
        &[
            ("拖拽", "任意方向平移画布，松手带惯性"),
            ("滚轮", "上下 / 左右移动，缓停"),
            ("Ctrl + =/-", "放大 / 缩小（以画面中心为锚点）"),
            ("Ctrl + 0", "回到 100%"),
            ("点击 +", "选图（有目录时打开图片列表）"),
            ("a / +", "选图：有目录翻列表（右侧实时预览）"),
            ("x / Delete", "移除鼠标下的图片"),
            ("悬停", "状态栏显示这张图的名字"),
            ("点击图片", "弹窗看单张"),
            ("Ctrl + T / W", "新建 / 关闭画布"),
            ("Ctrl + 1~9", "切换画布"),
            ("Esc", "退出界面（后台保留）"),
            ("Ctrl + Q", "连后台一起退出"),
        ]
    };
    let w = 56.min(content.width.saturating_sub(4)).max(30);
    let h = ((keys.len() as u16) + 4).min(content.height.saturating_sub(2));
    let rect = Rect {
        x: content.x + (content.width.saturating_sub(w)) / 2,
        y: content.y + (content.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);
    let mut lines = Vec::new();
    for (k, d) in keys {
        let pad = 14usize.saturating_sub(UnicodeWidthStr::width(*k));
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {k}{}", " ".repeat(pad)),
                Style::default().fg(t.accent),
            ),
            Span::styled(*d, Style::default().fg(t.muted)),
        ]));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.accent))
        .title(Span::styled(" ymi · keys ", Style::default().fg(t.accent)))
        .style(Style::default().bg(t.sel_bg));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

/// Where the picker's pieces live. Shared with the graphics layer, which
/// paints the preview into `preview`.
pub struct PickerLayout {
    pub rect: Rect,
    pub list: Rect,
    pub preview: Rect,
}

pub fn picker_layout(content: Rect) -> PickerLayout {
    let rect = modal_rect(content);
    if rect.width < 6 || rect.height < 5 {
        return PickerLayout {
            rect,
            list: Rect::default(),
            preview: Rect::default(),
        };
    }
    let inner = Rect {
        x: rect.x + 1,
        y: rect.y + 1,
        width: rect.width.saturating_sub(2),
        height: rect.height.saturating_sub(2),
    };
    let mut list = inner;
    let mut preview = Rect::default();
    // Only split the box when there is enough room for a picture worth seeing.
    if inner.width >= 84 && inner.height >= 10 {
        let pw = (((inner.width as u32) * 38) / 100).clamp(26, 60) as u16;
        list.width = inner.width - pw - 2;
        preview = Rect {
            x: inner.x + inner.width - pw,
            y: inner.y,
            width: pw,
            height: inner.height,
        };
    }
    PickerLayout {
        rect,
        list,
        preview,
    }
}

fn draw_picker(f: &mut Frame, app: &App, p: &Picker) {
    let t = &app.theme;
    let lay = picker_layout(app.content);
    if lay.rect.width < 6 || lay.rect.height < 5 {
        return;
    }
    f.render_widget(Clear, lay.rect);

    let hits = crate::scan::filter(&p.all, &p.query);
    let inner_h = lay.list.height as usize;
    let start = if p.sel >= inner_h {
        p.sel + 1 - inner_h
    } else {
        0
    };

    let mut lines = Vec::new();
    for (n, &idx) in hits.iter().enumerate().skip(start).take(inner_h) {
        let path = &p.all[idx];
        let rel = std::path::Path::new(path)
            .strip_prefix(&p.root)
            .map(|r| r.display().to_string())
            .unwrap_or_else(|_| path.clone());
        let style = if n == p.sel {
            Style::default()
                .fg(Color::Rgb(0x16, 0x18, 0x1e))
                .bg(t.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.muted)
        };
        lines.push(Line::from(Span::styled(
            format!(" {} ", fit(&rel, lay.list.width.saturating_sub(4) as usize)),
            style,
        )));
    }
    if hits.is_empty() {
        lines.push(Line::from(Span::styled(
            i18n::t("  没有匹配的图片", "  no matching pictures"),
            Style::default().fg(t.dim),
        )));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.accent))
        .title(Span::styled(
            format!(
                " 选择图片 · {} 张 · 画布 {} 张 ",
                hits.len(),
                app.session.tab().images.len()
            ),
            Style::default().fg(t.accent),
        ))
        .title_bottom(Span::styled(
            i18n::p(
                format!(" /{:<20} ↑↓ Enter 加入 · Esc 结束 ", p.query),
                format!(" /{:<20} ↑↓ Enter adds · Esc ends ", p.query),
            ),
            Style::default().fg(t.dim),
        ))
        .style(Style::default().bg(t.sel_bg));
    f.render_widget(Paragraph::new(lines).block(block), lay.rect);

    // Divider between the list and the preview. The picture itself is painted
    // over the cells on the right by the graphics layer.
    if lay.preview.width > 0 {
        let x = lay.preview.x - 1;
        for y in lay.preview.y..lay.preview.y + lay.preview.height {
            f.render_widget(
                Paragraph::new(Span::styled("│", Style::default().fg(t.faint))),
                Rect {
                    x,
                    y,
                    width: 1,
                    height: 1,
                },
            );
        }
    }
}

fn draw_input(f: &mut Frame, app: &App) {
    let t = &app.theme;
    let w = (app.content.width as f32 * 0.8) as u16;
    let h = 6u16.min(app.content.height);
    let rect = Rect {
        x: app.content.x + (app.content.width.saturating_sub(w)) / 2,
        y: app.content.y + (app.content.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(" › ", Style::default().fg(t.accent)),
            Span::styled(
                fit(&app.input, w.saturating_sub(6) as usize),
                Style::default().fg(t.muted),
            ),
            Span::styled("▌", Style::default().fg(t.accent)),
        ]),
        Line::from(Span::styled(
            if app.completions.is_empty() {
                i18n::t("  Tab 补全 · Enter 打开 / 添加 · Esc 取消", "  Tab completes · Enter opens / adds · Esc cancels")
            } else {
                i18n::t("  补全候选：", "  completions: ")
            },
            Style::default().fg(t.dim),
        )),
    ];
    for c in app.completions.iter().take(3) {
        lines.push(Line::from(Span::styled(
            format!("   {}", fit(c, w.saturating_sub(6) as usize)),
            Style::default().fg(t.muted),
        )));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.accent))
        .title(Span::styled(i18n::t(" 打开图片或目录 ", " open a picture or folder "), Style::default().fg(t.accent)))
        .style(Style::default().bg(t.sel_bg));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

fn draw_toast(f: &mut Frame, app: &App, msg: String) {
    let t = &app.theme;
    let w = (UnicodeWidthStr::width(msg.as_str()) as u16 + 4).min(app.content.width);
    let rect = Rect {
        x: app.content.x + (app.content.width.saturating_sub(w)) / 2,
        y: app.content.y + app.content.height.saturating_sub(2),
        width: w,
        height: 1,
    };
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(Span::styled(
            format!(" {msg} "),
            Style::default().fg(Color::Rgb(0x16, 0x18, 0x1e)).bg(t.muted),
        )),
        rect,
    );
}

fn fit(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > max.saturating_sub(1) {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

fn short_path(p: &str) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy().to_string();
        if let Some(rest) = p.strip_prefix(&home) {
            return format!("~{rest}");
        }
    }
    p.to_string()
}
