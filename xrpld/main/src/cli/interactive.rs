use console::Style;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{self, ClearType},
};
use std::io::{self, BufRead, Write, stdout};
use std::thread;
use std::time::Duration;

struct CommandDef {
    name: &'static str,
    description: &'static str,
    subcommands: Option<&'static [CommandDef]>,
}

const COMMANDS: &[CommandDef] = &[
    CommandDef {
        name: "status",
        description: "Node status overview",
        subcommands: None,
    },
    CommandDef {
        name: "health",
        description: "Health check",
        subcommands: None,
    },
    CommandDef {
        name: "peers",
        description: "Connected peers",
        subcommands: None,
    },
    CommandDef {
        name: "fee",
        description: "Current fee info",
        subcommands: None,
    },
    CommandDef {
        name: "ledger",
        description: "Ledger details (optional: seq number)",
        subcommands: None,
    },
    CommandDef {
        name: "account",
        description: "Account info (requires: address)",
        subcommands: None,
    },
    CommandDef {
        name: "sync-status",
        description: "Sync progress",
        subcommands: None,
    },
    CommandDef {
        name: "validators",
        description: "Trusted validators",
        subcommands: None,
    },
    CommandDef {
        name: "amendments",
        description: "Amendment status",
        subcommands: None,
    },
    CommandDef {
        name: "db-stats",
        description: "Database statistics",
        subcommands: None,
    },
    CommandDef {
        name: "log-level",
        description: "Get/set log level",
        subcommands: None,
    },
    CommandDef {
        name: "benchmark",
        description: "Run benchmarks",
        subcommands: None,
    },
    CommandDef {
        name: "validator-keys",
        description: "Manage validator keys",
        subcommands: Some(&[
            CommandDef {
                name: "generate",
                description: "Generate master keypair",
                subcommands: None,
            },
            CommandDef {
                name: "create-token",
                description: "Create validator token",
                subcommands: None,
            },
            CommandDef {
                name: "sign",
                description: "Sign data with master key",
                subcommands: None,
            },
            CommandDef {
                name: "revoke",
                description: "Revoke validator key",
                subcommands: None,
            },
            CommandDef {
                name: "show",
                description: "Display current key info",
                subcommands: None,
            },
        ]),
    },
    CommandDef {
        name: "stop",
        description: "Graceful node shutdown",
        subcommands: None,
    },
    CommandDef {
        name: "version",
        description: "Build version info",
        subcommands: None,
    },
    CommandDef {
        name: "doctor",
        description: "Pre-flight diagnostics",
        subcommands: None,
    },
    CommandDef {
        name: "config",
        description: "Validate config file",
        subcommands: None,
    },
    CommandDef {
        name: "clear",
        description: "Clear screen",
        subcommands: None,
    },
    CommandDef {
        name: "exit",
        description: "Exit interactive mode",
        subcommands: None,
    },
];

const MAX_VISIBLE: usize = 5;

const LOGO_LINES: &[&str] = &[
    r"██╗  ██╗██████╗ ██████╗ ██╗     ██████╗ ",
    r"╚██╗██╔╝██╔══██╗██╔══██╗██║     ██╔══██╗",
    r" ╚███╔╝ ██████╔╝██████╔╝██║     ██║  ██║",
    r" ██╔██╗ ██╔══██╗██╔═══╝ ██║     ██║  ██║",
    r"██╔╝ ██╗██║  ██║██║     ███████╗██████╔╝",
    r"╚═╝  ╚═╝╚═╝  ╚═╝╚═╝     ╚══════╝╚═════╝ ",
];

/// Drop guard to ensure raw mode is disabled on panic/exit.
struct RawModeGuard;

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(stdout(), cursor::Show);
    }
}

fn rust_orange() -> Style {
    Style::new().color256(166)
}

fn fuzzy_match(input: &str, target: &str) -> bool {
    if input.is_empty() {
        return true;
    }
    let input_lower = input.to_lowercase();
    let target_lower = target.to_lowercase();
    let mut chars = input_lower.chars().peekable();
    for tc in target_lower.chars() {
        if chars.peek() == Some(&tc) {
            chars.next();
        }
    }
    chars.peek().is_none()
}

fn filter_commands<'a>(input: &str, commands: &'a [CommandDef]) -> Vec<&'a CommandDef> {
    commands
        .iter()
        .filter(|c| fuzzy_match(input, c.name))
        .collect()
}

fn print_logo_colored(color: u8) {
    let (term_width, _) = terminal::size().unwrap_or((80, 24));
    let w = term_width as usize;
    let style = Style::new().color256(color);
    for line in LOGO_LINES {
        let pad = w.saturating_sub(line.chars().count()) / 2;
        println!("{}{}", " ".repeat(pad), style.apply_to(line));
    }
}

fn print_logo_gradient() {
    let (term_width, _) = terminal::size().unwrap_or((80, 24));
    let w = term_width as usize;
    // Gradient: bright orange top → deep rust bottom
    let colors: &[u8] = &[202, 202, 166, 166, 130, 88];
    for (i, line) in LOGO_LINES.iter().enumerate() {
        let color = colors[i.min(colors.len() - 1)];
        let style = Style::new().color256(color);
        let pad = w.saturating_sub(line.chars().count()) / 2;
        println!("{}{}", " ".repeat(pad), style.apply_to(line));
    }
}

fn show_animated_logo() {
    let mut stdout = stdout();
    let logo_height = LOGO_LINES.len() as u16;
    // Fade from gray → rust gradient
    let fade_colors: &[u8] = &[236, 240, 244, 130];

    println!("\n");
    print_logo_colored(fade_colors[0]);
    let _ = stdout.flush();

    for &color in &fade_colors[1..] {
        thread::sleep(Duration::from_millis(120));
        execute!(stdout, cursor::MoveUp(logo_height)).unwrap();
        print_logo_colored(color);
        let _ = stdout.flush();
    }

    // Final frame: full gradient
    thread::sleep(Duration::from_millis(120));
    execute!(stdout, cursor::MoveUp(logo_height)).unwrap();
    print_logo_gradient();
    let _ = stdout.flush();
}

fn show_welcome() {
    let (term_width, _) = terminal::size().unwrap_or((80, 24));
    let w = term_width as usize;
    let dim = Style::new().dim();

    show_animated_logo();

    let version = "v0.1.0";
    let vpad = w.saturating_sub(version.len()) / 2;
    println!("{}{}", " ".repeat(vpad), dim.apply_to(version));

    let tagline = "The fastest way to operate your XRPL node";
    let tpad = w.saturating_sub(tagline.len()) / 2;
    println!("{}{}", " ".repeat(tpad), dim.apply_to(tagline));
    println!();
}

/// prompt_row is the row (from top) where the prompt lives.
/// draw_ui moves to that row, clears down, and redraws prompt + suggestions below.
fn draw_ui(
    stdout: &mut io::Stdout,
    input: &str,
    filtered: &[&CommandDef],
    selected: usize,
    scroll_offset: usize,
    prompt_row: u16,
    from_history: bool,
    in_subcommand_mode: bool,
) -> io::Result<()> {
    let orange = rust_orange();
    let dim = Style::new().dim();

    // Move to prompt row and clear everything below
    execute!(stdout, cursor::MoveTo(0, prompt_row))?;
    execute!(stdout, terminal::Clear(ClearType::FromCursorDown))?;

    // Print prompt
    let prompt = format!("  {} {}", orange.apply_to("❯"), input);
    write!(stdout, "{}\r\n", prompt)?;

    // Show suggestions below prompt
    if !from_history && (!input.is_empty() || in_subcommand_mode) {
        if !filtered.is_empty() {
            let total = filtered.len();
            let visible = total.min(MAX_VISIBLE);
            let hidden_above = scroll_offset;
            let hidden_below = total.saturating_sub(scroll_offset + visible);

            if hidden_above > 0 {
                write!(
                    stdout,
                    "    {}\r\n",
                    dim.apply_to(format!("↑ {} more", hidden_above))
                )?;
            }

            for i in 0..visible {
                let idx = scroll_offset + i;
                let cmd = filtered[idx];
                if idx == selected {
                    let line = format!("    ▸ {:<16}{}", cmd.name, cmd.description);
                    write!(stdout, "{}\r\n", orange.apply_to(&line))?;
                } else {
                    write!(
                        stdout,
                        "      {:<16}{}\r\n",
                        cmd.name,
                        dim.apply_to(cmd.description)
                    )?;
                }
            }

            if hidden_below > 0 {
                write!(
                    stdout,
                    "    {}\r\n",
                    dim.apply_to(format!("↓ {} more", hidden_below))
                )?;
            }
        } else {
            write!(stdout, "    {}\r\n", dim.apply_to("no commands found"))?;
        }
    }

    // Move cursor back to end of input on prompt line
    execute!(stdout, cursor::MoveTo((4 + input.len()) as u16, prompt_row))?;
    stdout.flush()?;
    Ok(())
}

fn read_line_cooked(prompt: &str) -> String {
    let dim = Style::new().dim();
    print!("{}", dim.apply_to(prompt));
    let _ = stdout().flush();
    let mut buf = String::new();
    let _ = io::stdin().lock().read_line(&mut buf);
    buf.trim().to_string()
}

fn dispatch_command(url: &str, name: &str, arg: &str) {
    match name {
        "status" => super::status::run(url),
        "health" => {
            let _ = super::health::run(url);
        }
        "peers" => super::peers::run(url),
        "fee" => super::fee::run(url),
        "ledger" => {
            let seq = arg.parse::<u64>().ok();
            super::ledger_cmd::run(url, seq);
        }
        "account" => {
            if arg.is_empty() {
                eprintln!(
                    "  {} Usage: account <address>",
                    Style::new().red().apply_to("●")
                );
            } else {
                super::account::run(url, arg);
            }
        }
        "sync-status" => super::sync_status::run(url),
        "validators" => super::validators::run(url),
        "amendments" => super::amendments::run(url),
        "db-stats" => super::db_stats::run(url),
        "log-level" => {
            super::log_level::run(url, if arg.is_empty() { None } else { Some(arg) });
        }
        "benchmark" => super::benchmark::run(),
        "stop" => super::stop::run(url),
        "version" => super::version::run(),
        "doctor" => super::doctor::run(url, None),
        "config" => super::config_check::run(None),
        "clear" => {
            print!("\x1B[2J\x1B[1;1H");
            let _ = stdout().flush();
        }
        "generate" => super::validator_keys::run_generate(),
        "create-token" => super::validator_keys::run_create_token(None),
        "sign" => {
            if arg.is_empty() {
                eprintln!("  {} Usage: sign <data>", Style::new().red().apply_to("●"));
            } else {
                super::validator_keys::run_sign(arg);
            }
        }
        "revoke" => super::validator_keys::run_revoke(),
        "show" => super::validator_keys::run_show(),
        _ => eprintln!("  Unknown command: {}", name),
    }
}

pub fn run(url: &str) {
    let _guard = RawModeGuard;

    // Clear screen and show animated logo + welcome
    print!("\x1B[2J\x1B[1;1H");
    let _ = stdout().flush();
    show_welcome();

    // Record the prompt row (current cursor position)
    let mut prompt_row = cursor::position().map(|(_, r)| r).unwrap_or(12);

    terminal::enable_raw_mode().expect("Failed to enable raw mode");
    let mut stdout = stdout();
    let _ = execute!(stdout, cursor::Hide);

    let mut input = String::new();
    let mut selected: usize = 0;
    let mut scroll_offset: usize = 0;
    let mut history: Vec<String> = Vec::new();
    let mut history_idx: Option<usize> = None;
    let mut from_history = false;
    let mut in_subcommands: Option<&'static [CommandDef]> = None;

    loop {
        let commands: &[CommandDef] = in_subcommands.unwrap_or(COMMANDS);
        let filtered = filter_commands(&input, commands);
        if selected >= filtered.len() && !filtered.is_empty() {
            selected = filtered.len() - 1;
        }
        if !filtered.is_empty() {
            let visible = filtered.len().min(MAX_VISIBLE);
            if selected < scroll_offset {
                scroll_offset = selected;
            } else if selected >= scroll_offset + visible {
                scroll_offset = selected + 1 - visible;
            }
            if scroll_offset + visible > filtered.len() {
                scroll_offset = filtered.len().saturating_sub(visible);
            }
        } else {
            scroll_offset = 0;
        }

        let _ = draw_ui(
            &mut stdout,
            &input,
            &filtered,
            selected,
            scroll_offset,
            prompt_row,
            from_history,
            in_subcommands.is_some(),
        );

        let evt = match event::read() {
            Ok(e) => e,
            Err(_) => break,
        };

        match evt {
            Event::Key(KeyEvent {
                code, modifiers, ..
            }) => {
                if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
                    break;
                }
                match code {
                    KeyCode::Char(c) => {
                        input.push(c);
                        selected = 0;
                        scroll_offset = 0;
                        history_idx = None;
                        from_history = false;
                    }
                    KeyCode::Backspace => {
                        input.pop();
                        selected = 0;
                        scroll_offset = 0;
                        history_idx = None;
                        from_history = false;
                    }
                    KeyCode::Up => {
                        if from_history || (input.is_empty() && !history.is_empty()) {
                            let idx = match history_idx {
                                Some(i) if i > 0 => i - 1,
                                Some(i) => i,
                                None => history.len() - 1,
                            };
                            history_idx = Some(idx);
                            input = history[idx].clone();
                            from_history = true;
                        } else if !filtered.is_empty() {
                            selected = if selected == 0 {
                                filtered.len() - 1
                            } else {
                                selected - 1
                            };
                        }
                    }
                    KeyCode::Down => {
                        if from_history {
                            let idx = history_idx.unwrap_or(0);
                            if idx + 1 < history.len() {
                                history_idx = Some(idx + 1);
                                input = history[idx + 1].clone();
                            } else {
                                history_idx = None;
                                input.clear();
                                from_history = false;
                            }
                        } else if !filtered.is_empty() {
                            selected = (selected + 1) % filtered.len();
                        }
                    }
                    KeyCode::Tab => {
                        if !filtered.is_empty() {
                            input = filtered[selected].name.to_string();
                        }
                    }
                    KeyCode::Esc => {
                        if in_subcommands.is_some() {
                            in_subcommands = None;
                            input.clear();
                            selected = 0;
                            scroll_offset = 0;
                        } else {
                            input.clear();
                            selected = 0;
                            scroll_offset = 0;
                            history_idx = None;
                        }
                    }
                    KeyCode::Enter => {
                        if filtered.is_empty() {
                            continue;
                        }
                        let cmd = filtered[selected];
                        let cmd_name = cmd.name.to_string();

                        if let Some(subs) = cmd.subcommands {
                            in_subcommands = Some(subs);
                            input.clear();
                            selected = 0;
                            scroll_offset = 0;
                            continue;
                        }

                        let arg = input
                            .strip_prefix(&cmd_name)
                            .unwrap_or("")
                            .trim()
                            .to_string();

                        let _ = execute!(stdout, cursor::Show);
                        let _ = terminal::disable_raw_mode();

                        // Clear suggestion area and move below prompt
                        execute!(stdout, cursor::MoveTo(0, prompt_row)).unwrap();
                        execute!(stdout, terminal::Clear(ClearType::FromCursorDown)).unwrap();
                        println!();

                        if cmd_name == "exit" {
                            return;
                        }

                        let final_arg =
                            if arg.is_empty() && matches!(cmd_name.as_str(), "account" | "sign") {
                                read_line_cooked("  argument: ")
                            } else {
                                arg
                            };

                        dispatch_command(url, &cmd_name, &final_arg);

                        history.push(cmd_name.clone());

                        println!();
                        super::command_divider();
                        println!();
                        // Update prompt_row to current cursor position after output
                        prompt_row = cursor::position().map(|(_, r)| r).unwrap_or(prompt_row);
                        terminal::enable_raw_mode().expect("Failed to re-enable raw mode");
                        let _ = execute!(stdout, cursor::Hide);
                        input.clear();
                        selected = 0;
                        scroll_offset = 0;
                        in_subcommands = None;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}
