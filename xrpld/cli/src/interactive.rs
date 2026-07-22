use console::Style;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, ClearType},
};
use std::io::{self, Write};

const MAX_VISIBLE: usize = 6;
const AMBER: Color = Color::Rgb {
    r: 255,
    g: 176,
    b: 0,
};
const AMBER_DIM: Color = Color::Rgb {
    r: 180,
    g: 130,
    b: 50,
};

struct Command {
    name: &'static str,
    desc: &'static str,
}

const COMMANDS: &[Command] = &[
    Command {
        name: "status",
        desc: "Node status overview",
    },
    Command {
        name: "health",
        desc: "Health check",
    },
    Command {
        name: "peers",
        desc: "Connected peers",
    },
    Command {
        name: "fee",
        desc: "Fee information",
    },
    Command {
        name: "ledger",
        desc: "Ledger info",
    },
    Command {
        name: "account",
        desc: "Account info",
    },
    Command {
        name: "sync-status",
        desc: "Sync progress",
    },
    Command {
        name: "validators",
        desc: "Trusted validators",
    },
    Command {
        name: "amendments",
        desc: "Amendment status",
    },
    Command {
        name: "db-stats",
        desc: "Database statistics",
    },
    Command {
        name: "benchmark",
        desc: "Run benchmarks",
    },
    Command {
        name: "validator-keys",
        desc: "Manage validator keys",
    },
    Command {
        name: "doctor",
        desc: "Pre-flight diagnostics",
    },
    Command {
        name: "version",
        desc: "Show version",
    },
    Command {
        name: "stop",
        desc: "Stop the node",
    },
    Command {
        name: "exit",
        desc: "Exit CLI",
    },
];

fn filter_commands(input: &str) -> Vec<usize> {
    if input.is_empty() {
        return Vec::new();
    }
    let lower = input.to_lowercase();
    let mut results: Vec<(usize, bool)> = COMMANDS
        .iter()
        .enumerate()
        .filter(|(_, c)| c.name.contains(&lower))
        .map(|(i, c)| (i, c.name.starts_with(&lower)))
        .collect();
    results.sort_by(|a, b| b.1.cmp(&a.1));
    results.into_iter().map(|(i, _)| i).collect()
}

/// Ensure there are enough blank lines below cursor for suggestions.
/// Returns the row where the prompt should be drawn.
fn ensure_space(stdout: &mut impl Write, lines_needed: u16) -> u16 {
    let (_, term_h) = terminal::size().unwrap_or((80, 24));
    let (_, cur_row) = cursor::position().unwrap_or((0, 10));

    let space_below = term_h.saturating_sub(cur_row + 1);
    if space_below < lines_needed {
        // Scroll by printing newlines
        let deficit = lines_needed - space_below;
        for _ in 0..deficit {
            let _ = queue!(stdout, Print("\n"));
        }
        let _ = stdout.flush();
        // Prompt row is now higher
        cur_row.saturating_sub(deficit)
    } else {
        cur_row
    }
}

fn render(
    stdout: &mut impl Write,
    input: &str,
    matches: &[usize],
    selected: usize,
    scroll_offset: usize,
    prompt_row: &mut u16,
) {
    let total = matches.len();
    let visible = total.saturating_sub(scroll_offset).min(MAX_VISIBLE);
    let has_more_below = scroll_offset + visible < total;
    let has_more_above = scroll_offset > 0;
    let extra = if has_more_below { 1 } else { 0 } + if has_more_above { 1 } else { 0 };
    let divider_lines: u16 = if visible > 0 { 2 } else { 0 }; // top + bottom divider
    let lines_needed = (visible + extra) as u16 + divider_lines;

    // Ensure space and get correct prompt row
    *prompt_row = ensure_space(stdout, lines_needed + 1);

    // Move to prompt row and clear everything from there down
    let _ = queue!(stdout, cursor::MoveTo(0, *prompt_row));
    let _ = queue!(stdout, terminal::Clear(ClearType::FromCursorDown));

    // Draw prompt
    let _ = queue!(stdout, SetForegroundColor(AMBER));
    let _ = queue!(stdout, Print("  ❯ "));
    let _ = queue!(stdout, ResetColor);
    let _ = queue!(stdout, Print(input));

    // Ghost hint
    if !matches.is_empty() && !input.is_empty() {
        let first = COMMANDS[matches[0]].name;
        if first.starts_with(&input.to_lowercase()) && first != input {
            let _ = queue!(stdout, SetForegroundColor(Color::DarkGrey));
            let _ = queue!(stdout, Print(&first[input.len()..]));
            let _ = queue!(stdout, ResetColor);
        }
    }

    // Draw suggestions with dividers
    if visible > 0 {
        let term_width = terminal::size().map(|(w, _)| w as usize).unwrap_or(80);
        let div_w = (term_width * 9) / 10;
        let mut row_offset: u16 = 1;

        // Top divider
        let _ = queue!(stdout, cursor::MoveTo(0, *prompt_row + row_offset));
        let _ = queue!(
            stdout,
            SetForegroundColor(Color::Rgb {
                r: 60,
                g: 60,
                b: 60
            })
        );
        let _ = queue!(stdout, Print(format!("  {}", "─".repeat(div_w))));
        let _ = queue!(stdout, ResetColor);
        row_offset += 1;

        if has_more_above {
            let _ = queue!(stdout, cursor::MoveTo(0, *prompt_row + row_offset));
            let _ = queue!(stdout, SetForegroundColor(Color::DarkGrey));
            let _ = queue!(stdout, Print(format!("    ↑ {} above", scroll_offset)));
            let _ = queue!(stdout, ResetColor);
            row_offset += 1;
        }

        for (i, &cmd_idx) in matches.iter().skip(scroll_offset).take(visible).enumerate() {
            let cmd = &COMMANDS[cmd_idx];
            let is_selected = (scroll_offset + i) == selected;
            let _ = queue!(
                stdout,
                cursor::MoveTo(0, *prompt_row + row_offset + i as u16)
            );

            if is_selected {
                let _ = queue!(stdout, SetForegroundColor(AMBER));
                let _ = queue!(stdout, Print(format!("  ▸ {:<16}", cmd.name)));
                let _ = queue!(stdout, SetForegroundColor(AMBER_DIM));
            } else {
                let _ = queue!(stdout, Print("    "));
                let _ = queue!(stdout, SetForegroundColor(Color::White));
                let _ = queue!(stdout, Print(format!("{:<16}", cmd.name)));
                let _ = queue!(stdout, SetForegroundColor(Color::DarkGrey));
            }
            let max_desc = term_width.saturating_sub(22);
            let _ = queue!(stdout, Print(&cmd.desc[..cmd.desc.len().min(max_desc)]));
            let _ = queue!(stdout, ResetColor);
        }

        let mut after_row = *prompt_row + row_offset + visible as u16;

        if has_more_below {
            let _ = queue!(stdout, cursor::MoveTo(0, after_row));
            let _ = queue!(stdout, SetForegroundColor(Color::DarkGrey));
            let _ = queue!(
                stdout,
                Print(format!("    ↓ {} more", total - scroll_offset - visible))
            );
            let _ = queue!(stdout, ResetColor);
            after_row += 1;
        }

        // Bottom divider
        let _ = queue!(stdout, cursor::MoveTo(0, after_row));
        let _ = queue!(
            stdout,
            SetForegroundColor(Color::Rgb {
                r: 60,
                g: 60,
                b: 60
            })
        );
        let _ = queue!(stdout, Print(format!("  {}", "─".repeat(div_w))));
        let _ = queue!(stdout, ResetColor);
    }

    // Cursor back to end of input on prompt line
    let col = 4 + input.len() as u16;
    let _ = queue!(stdout, cursor::MoveTo(col, *prompt_row));
    let _ = stdout.flush();
}

pub fn run(url: &str) {
    super::logo::print_logo();

    let dim = Style::new().dim();
    println!(
        "  {}",
        dim.apply_to("Type to filter. ↑↓ select. Tab complete. Enter run. Ctrl+C exit.")
    );
    println!();

    // Panic hook
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), cursor::Show);
        original_hook(info);
    }));

    if terminal::enable_raw_mode().is_err() {
        eprintln!("  Cannot enter interactive mode. Use individual commands instead.");
        return;
    }

    let mut stdout = io::stdout();
    let _ = execute!(stdout, cursor::Show);

    let mut input = String::new();
    let mut matches: Vec<usize> = Vec::new();
    let mut selected: usize = 0;
    let mut scroll_offset: usize = 0;
    let mut prompt_row: u16 = cursor::position().map(|(_, r)| r).unwrap_or(10);
    let mut history: Vec<String> = Vec::new();
    let mut history_idx: Option<usize> = None;

    render(
        &mut stdout,
        &input,
        &matches,
        selected,
        scroll_offset,
        &mut prompt_row,
    );

    loop {
        if !event::poll(std::time::Duration::from_millis(50)).unwrap_or(false) {
            continue;
        }
        let evt = match event::read() {
            Ok(e) => e,
            Err(_) => break,
        };

        match evt {
            Event::Key(KeyEvent {
                code, modifiers, ..
            }) => {
                match code {
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        if input.is_empty() {
                            break;
                        }
                        input.clear();
                        matches.clear();
                        selected = 0;
                        scroll_offset = 0;
                        history_idx = None;
                    }
                    KeyCode::Char('d') if modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Char(c) => {
                        input.push(c);
                        matches = filter_commands(&input);
                        selected = 0;
                        scroll_offset = 0;
                        history_idx = None;
                    }
                    KeyCode::Backspace => {
                        input.pop();
                        matches = filter_commands(&input);
                        selected = 0;
                        scroll_offset = 0;
                        history_idx = None;
                    }
                    KeyCode::Tab => {
                        if !matches.is_empty() {
                            input = COMMANDS[matches[selected]].name.to_string();
                            matches = filter_commands(&input);
                            selected = 0;
                            scroll_offset = 0;
                        }
                    }
                    KeyCode::Down => {
                        if !matches.is_empty() && selected + 1 < matches.len() {
                            selected += 1;
                            if selected >= scroll_offset + MAX_VISIBLE {
                                scroll_offset = selected + 1 - MAX_VISIBLE;
                            }
                        }
                    }
                    KeyCode::Up => {
                        if !matches.is_empty() {
                            if selected > 0 {
                                selected -= 1;
                                if selected < scroll_offset {
                                    scroll_offset = selected;
                                }
                            }
                        } else if input.is_empty() && !history.is_empty() {
                            let idx = history_idx
                                .map(|i| i.saturating_sub(1))
                                .unwrap_or(history.len() - 1);
                            history_idx = Some(idx);
                            input = history[idx].clone();
                            matches = filter_commands(&input);
                            selected = 0;
                            scroll_offset = 0;
                        }
                    }
                    KeyCode::Esc => {
                        input.clear();
                        matches.clear();
                        selected = 0;
                        scroll_offset = 0;
                        history_idx = None;
                    }
                    KeyCode::Enter => {
                        let cmd_name = if !matches.is_empty() {
                            COMMANDS[matches[selected]].name.to_string()
                        } else if !input.is_empty() {
                            input.clone()
                        } else {
                            render(
                                &mut stdout,
                                &input,
                                &matches,
                                selected,
                                scroll_offset,
                                &mut prompt_row,
                            );
                            continue;
                        };

                        // Clear and show what was executed
                        let _ = execute!(stdout, cursor::MoveTo(0, prompt_row));
                        let _ = execute!(stdout, terminal::Clear(ClearType::FromCursorDown));
                        let _ = execute!(stdout, SetForegroundColor(AMBER));
                        let _ = execute!(stdout, Print("  ❯ "));
                        let _ = execute!(stdout, ResetColor);
                        let _ = execute!(stdout, Print(&cmd_name));
                        let _ = execute!(stdout, Print("\r\n\r\n"));

                        if cmd_name == "exit" || cmd_name == "quit" {
                            break;
                        }

                        history.push(cmd_name.clone());

                        // Execute
                        let _ = terminal::disable_raw_mode();
                        dispatch_command(url, &cmd_name);
                        let term_w = terminal::size().map(|(w, _)| w as usize).unwrap_or(80);
                        println!();
                        let sep: String = "╌".repeat((term_w * 9) / 10);
                        println!("  \x1B[38;5;237m{}\x1B[0m", sep);
                        println!();

                        if terminal::enable_raw_mode().is_err() {
                            break;
                        }

                        // Reset
                        input.clear();
                        matches.clear();
                        selected = 0;
                        scroll_offset = 0;
                        history_idx = None;
                        prompt_row = cursor::position().map(|(_, r)| r).unwrap_or(prompt_row);
                    }
                    _ => {}
                }
                render(
                    &mut stdout,
                    &input,
                    &matches,
                    selected,
                    scroll_offset,
                    &mut prompt_row,
                );
            }
            Event::Resize(_, _) => {
                render(
                    &mut stdout,
                    &input,
                    &matches,
                    selected,
                    scroll_offset,
                    &mut prompt_row,
                );
            }
            _ => {}
        }
    }

    // Cleanup
    let _ = execute!(stdout, cursor::MoveTo(0, prompt_row));
    let _ = execute!(stdout, terminal::Clear(ClearType::FromCursorDown));
    let _ = terminal::disable_raw_mode();
    println!();
}

fn dispatch_command(url: &str, name: &str) {
    match name {
        "status" => {
            let _ = super::status::run(url);
        }
        "health" => {
            let _ = super::health::run(url);
        }
        "peers" => {
            let _ = super::peers::run(url);
        }
        "fee" => {
            let _ = super::fee::run(url);
        }
        "ledger" => {
            let _ = super::ledger_cmd::run(url, None);
        }
        "account" => {
            print!("  Account address: ");
            io::stdout().flush().ok();
            let mut addr = String::new();
            io::stdin().read_line(&mut addr).ok();
            let addr = addr.trim();
            if addr.is_empty() {
                eprintln!("  {} No address provided", Style::new().red().apply_to("●"));
            } else {
                let _ = super::account::run(url, addr);
            }
        }
        "sync-status" => {
            let _ = super::sync_status::run(url);
        }
        "validators" => {
            let _ = super::validators::run(url);
        }
        "amendments" => {
            let _ = super::amendments::run(url);
        }
        "db-stats" => {
            let _ = super::db_stats::run(url, None);
        }
        "benchmark" => super::benchmark::run(),
        "validator-keys" => super::validator_keys::run_show(),
        "doctor" => super::doctor::run(url, None),
        "version" => super::version::run(),
        "stop" => {
            let _ = super::stop::run(url);
        }
        _ => {
            eprintln!(
                "  {} Unknown command: {}",
                Style::new().red().apply_to("●"),
                name
            );
        }
    }
}
