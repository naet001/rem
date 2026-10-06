mod agent;
mod config;
mod context;
mod history;
mod markdown;
mod modes;
mod permissions;
mod popup;
mod sessions;
mod skills;
mod theme;
mod tools;
mod tui;

use agent::{AgentLoop, RigAgent};
use config::Config;
use tui::{RatatuiBackend, TuiBackend};

/// `rem --help` text: flags, config, sessions, in-TUI pointers.
const HELP: &str = "rem \u{2014} a terminal coding agent you can audit.

Usage:
  rem                    start a new session (TUI)
  rem --resume <id>      resume a saved session (TUI)
  rem --help, -h         show this help and exit
  rem --version, -V      show version and exit

Config: ~/.config/rem/config.toml (see config.example.toml).
  active_provider + [providers.*] entries; keys use env:VAR indirection \u{2014}
  export OPENAI_API_KEY / ANTHROPIC_API_KEY to match.

Sessions persist per turn in SQLite under your platform data dir; on exit
rem prints `Session saved. Resume with: rem --resume <id>`.

Inside the TUI: /quit /new /help /theme /init /resume /rename /fork
/skills /skill:<name> /model /effort /context, Shift+Tab cycles modes,
@path inlines file/dir contents. Docs: README.md, docs/adr/.";

/// CLI shape: run (fresh or resume) vs immediate stdout actions.
#[derive(Debug, PartialEq, Eq)]
enum Cli {
    Run { resume_id: Option<String> },
    Help,
    Version,
}

/// Parse `std::env::args` without new deps. `--help`/`--version` win over
/// everything; unknown `--flags` are fatal; bare positionals are ignored
/// (reserved for future subcommands like `exec`).
fn parse_cli(args: &[String]) -> Result<Cli, String> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return Ok(Cli::Help);
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        return Ok(Cli::Version);
    }
    let mut resume_id: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--resume" => {
                if i + 1 >= args.len() {
                    return Err("--resume requires a session id".to_string());
                }
                resume_id = Some(args[i + 1].clone());
                i += 2;
            }
            flag if flag.starts_with('-') => {
                return Err(format!("unknown flag {flag} (see rem --help)"));
            }
            _ => {
                i += 1;
            }
        }
    }
    Ok(Cli::Run { resume_id })
}

#[tokio::main]
async fn main() {
    // Flags that never touch config, db, or the TUI.
    let args: Vec<String> = std::env::args().collect();
    let resume_id: Option<String> = match parse_cli(&args) {
        Ok(Cli::Help) => {
            println!("{HELP}");
            return;
        }
        Ok(Cli::Version) => {
            println!("rem {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Ok(Cli::Run { resume_id }) => resume_id,
        Err(e) => {
            eprintln!("rem: {e}");
            std::process::exit(2);
        }
    };

    let cfg = match Config::from_file() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("rem: {e}");
            eprintln!(
                "hint: create ~/.config/rem/config.toml with active_provider + [providers.*] sections (see config.example.toml)"
            );
            std::process::exit(1);
        }
    };

    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let project_root = cwd.canonicalize().unwrap_or(cwd);
    let project_root_str = project_root.to_string_lossy().to_string();

    let conn = match sessions::open() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("rem: {e}");
            std::process::exit(1);
        }
    };
    let db_path_str = sessions::db_path()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    // New session or resume; resume of an unknown id is fatal.
    let is_resume = resume_id.is_some();
    let mut session = match resume_id {
        Some(id) => match sessions::get_session(&conn, &id) {
            Ok(Some(s)) => s,
            Ok(None) => {
                eprintln!("rem: no session with id {id}");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("rem: {e}");
                std::process::exit(1);
            }
        },
        None => {
            let tag = match cfg.active() {
                Ok(r) => format!("{}:{}", r.name, r.model),
                Err(e) => {
                    eprintln!("rem: {e}");
                    std::process::exit(1);
                }
            };
            match sessions::create_session(&conn, &project_root_str, &tag) {
                Ok(s) => {
                    if let Err(e) = sessions::update_title(&conn, &s.id, "untitled") {
                        eprintln!("rem: {e}");
                        std::process::exit(1);
                    }
                    sessions::get_session(&conn, &s.id)
                        .ok()
                        .flatten()
                        .unwrap_or(s)
                }
                Err(e) => {
                    eprintln!("rem: {e}");
                    std::process::exit(1);
                }
            }
        }
    };
    // Keep the in-memory copy consistent even if the refetch above failed.
    if session.title != "untitled" && !is_resume {
        session.title = "untitled".to_string();
    }
    drop(conn);

    let (approval_tx, approval_rx) = permissions::approval_channel();

    let agent = match RigAgent::new(&cfg, approval_tx, project_root) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("rem: {e}");
            std::process::exit(1);
        }
    };

    // Preload resumed history before the TUI owns the agent.
    if let Err(e) = agent.import_messages_json(&session.messages_json).await {
        eprintln!("rem: failed to restore session history: {e}");
        std::process::exit(1);
    }

    // TUI signature unchanged (src/tui.rs untouched): stash the session
    // coordinates where phase 3 TUI code can read them.
    unsafe {
        std::env::set_var("REM_SESSION_ID", &session.id);
        std::env::set_var("REM_SESSIONS_DB", &db_path_str);
        std::env::set_var("REM_SESSION_TITLE", &session.title);
    }

    let run_result = RatatuiBackend::new().run(agent, approval_rx);

    // Post-exit cleanup (all quit paths: /quit, Ctrl+D on empty input):
    // run() already tore down raw mode + the inline viewport, so wipe
    // visible screen + scrollback (mirrors launch Clear All+Purge) leaving
    // only the resume hint. Env var wins: /resume//fork inside the TUI
    // sync the final id there before returning.
    {
        use crossterm::{
            cursor::MoveTo,
            execute,
            terminal::{Clear, ClearType},
        };
        let _ = execute!(
            std::io::stdout(),
            Clear(ClearType::All),
            Clear(ClearType::Purge),
            MoveTo(0, 0),
        );
    }
    let sid = std::env::var("REM_SESSION_ID").unwrap_or_default();
    let sid = if sid.is_empty() {
        session.id.clone()
    } else {
        sid
    };
    if !sid.is_empty() {
        println!("Session saved. Resume with: rem --resume {sid}");
    }

    if let Err(e) = run_result {
        eprintln!("rem: TUI error: {e:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &[&str]) -> Vec<String> {
        std::iter::once("rem".to_string())
            .chain(words.iter().map(|w| w.to_string()))
            .collect()
    }

    #[test]
    fn bare_run_has_no_resume() {
        assert_eq!(parse_cli(&argv(&[])), Ok(Cli::Run { resume_id: None }));
    }

    #[test]
    fn resume_captures_id() {
        assert_eq!(
            parse_cli(&argv(&["--resume", "abc123"])),
            Ok(Cli::Run {
                resume_id: Some("abc123".to_string())
            })
        );
    }

    #[test]
    fn resume_without_id_is_fatal() {
        assert!(parse_cli(&argv(&["--resume"])).is_err());
    }

    #[test]
    fn help_and_version_win() {
        assert_eq!(parse_cli(&argv(&["--help"])), Ok(Cli::Help));
        assert_eq!(parse_cli(&argv(&["-h"])), Ok(Cli::Help));
        assert_eq!(parse_cli(&argv(&["--version"])), Ok(Cli::Version));
        assert_eq!(parse_cli(&argv(&["-V"])), Ok(Cli::Version));
        // Even alongside other flags.
        assert_eq!(
            parse_cli(&argv(&["--resume", "x", "--help"])),
            Ok(Cli::Help)
        );
    }

    #[test]
    fn unknown_flag_is_fatal_with_hint() {
        let err = parse_cli(&argv(&["--yolo"])).unwrap_err();
        assert!(err.contains("unknown flag"), "got: {err}");
        assert!(err.contains("--help"), "got: {err}");
    }

    #[test]
    fn bare_positionals_ignored_for_future_subcommands() {
        assert_eq!(
            parse_cli(&argv(&["exec", "do stuff"])),
            Ok(Cli::Run { resume_id: None })
        );
    }
}
