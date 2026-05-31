#[allow(dead_code)]
use console::Style;
use std::io::{Write, stdout};
use std::thread;
use std::time::Duration;

#[allow(dead_code)]
const LOGO_LINES: &[&str] = &[
    r"██╗  ██╗██████╗ ██████╗ ██╗     ██████╗ ",
    r"╚██╗██╔╝██╔══██╗██╔══██╗██║     ██╔══██╗",
    r" ╚███╔╝ ██████╔╝██████╔╝██║     ██║  ██║",
    r" ██╔██╗ ██╔══██╗██╔═══╝ ██║     ██║  ██║",
    r"██╔╝ ██╗██║  ██║██║     ███████╗██████╔╝",
    r"╚═╝  ╚═╝╚═╝  ╚═╝╚═╝     ╚══════╝╚═════╝ ",
];

fn center_pad() -> String {
    let term_width = terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(80);
    let logo_width = 42;
    let pad = term_width.saturating_sub(logo_width) / 2;
    " ".repeat(pad)
}

#[allow(dead_code)]
pub fn print_logo() {
    print_logo_animated(true);
}

#[allow(dead_code)]
pub fn print_logo_static() {
    print_logo_animated(false);
}

fn print_logo_animated(animate: bool) {
    let colors: &[u8] = &[202, 202, 166, 166, 130, 88];
    let pad = center_pad();
    let mut out = stdout();

    println!();
    for (i, line) in LOGO_LINES.iter().enumerate() {
        let style = Style::new().color256(colors[i.min(colors.len() - 1)]);
        println!("{}{}", pad, style.apply_to(line));
        if animate {
            out.flush().ok();
            thread::sleep(Duration::from_millis(40));
        }
    }

    let dim = Style::new().dim();
    let version_line = format!("v{}", env!("CARGO_PKG_VERSION"));
    let tagline = "The fastest way to operate your XRPL node";

    let term_width = terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(80);
    let ver_pad = " ".repeat(term_width.saturating_sub(version_line.len()) / 2);
    let tag_pad = " ".repeat(term_width.saturating_sub(tagline.len()) / 2);

    println!("{}{}", ver_pad, dim.apply_to(&version_line));
    println!("{}{}", tag_pad, dim.apply_to(tagline));
    println!();
}
