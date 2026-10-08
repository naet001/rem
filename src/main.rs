mod agent;
mod auto_rules;
mod config;
mod context;
mod fs_guard;
mod headless;
mod history;
mod markdown;
mod mcp;
mod modes;
mod permissions;
mod popup;
mod rules;
mod sandbox;
mod sessions;
mod settings;
mod skills;
mod theme;
mod tools;
mod trust;
mod tui;

use agent::{AgentLoop, RigAgent};
use config::Config;
use headless::OutputFormat;
use modes::Mode;
use tui::{RatatuiBackend, TuiBackend};

/// `rem --help` text: flags, config, sessions, in-TUI pointers.
const HELP: &str = "rem — a terminal coding agent you can audit.

Usage:
  rem                    start a new session (TUI)
  rem --resume <id>      resume a saved session (TUI)
  rem -p \"<prompt>\"       run one turn headless, print reply to stdout
  rem --help, -h         show this help and exit
  rem --version, -V      show version and exit

Headless (all flags require -p/--print):
  -p, --print [\"<prompt>\"]  enable print mode; prompt is trailing args.
                            pipe stdin too (echo ... | rem -p \"...\");
                            stdin first + -p second, either alone works.
  --resume <id>          continue a saved session headless
  --mode <name>          auto | default(manual) | acceptEdits | plan | dontAsk | bypassPermissions (default manual)
  --allow-all            auto-approve Confirm verdicts (Deny floor still holds)
  --output <fmt>         text (default) | json | jsonl event stream

Config: ~/.config/rem/config.toml (see config.example.toml).
  active_provider + [providers.*] entries; keys use env:VAR indirection —
  export OPENAI_API_KEY / ANTHROPIC_API_KEY to match.

Sessions persist per turn in SQLite under your platform data dir; on exit
rem prints `Session saved. Resume with: rem --resume <id>` (stderr headless).

Inside the TUI: /quit /new /help /theme /init /resume /rename /fork
/skills /skill:<name> /model /effort /context, Shift+Tab cycles modes,
@path inlines file/dir contents. Docs: README.md, docs/adr/.";

/// CLI shape: TUI run (fresh or resume) vs headless print vs immediate
/// stdout actions. Headless is `-p` only (grilled P0-#2: no `exec`).
#[derive(Debug, PartialEq, Eq)]
enum Cli {
    Run {
        resume_id: Option<String>,
    },
    Print {
        prompt: Option<String>,
        resume_id: Option<String>,
        mode: Option<Mode>,
        allow_all: bool,
        output: OutputFormat,
    },
    Help,
    Version,
}

/// Parse `std::env::args` without new deps. `--help`/`--version` win over
/// everything; unknown `--flags` are fatal. `-p`/`--print` switches to
/// headless: trailing bare args join into the prompt (stdin supplies the
/// rest). `--mode` / `--allow-all` / `--output` require `-p`.
fn parse_cli(args: &[String]) -> Result<Cli, String> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return Ok(Cli::Help);
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        return Ok(Cli::Version);
    }
    let mut print_mode = false;
    let mut prompt_parts: Vec<String> = Vec::new();
    let mut resume_id: Option<String> = None;
    let mut mode: Option<Mode> = None;
    let mut allow_all = false;
    let mut output = OutputFormat::Text;
    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "-p" || a == "--print" {
            print_mode = true;
            i += 1;
        } else if let Some(v) = a.strip_prefix("--print=") {
            print_mode = true;
            if !v.is_empty() {
                prompt_parts.push(v.to_string());
            }
            i += 1;
        } else if let Some(v) = a.strip_prefix("-p=") {
            print_mode = true;
            if !v.is_empty() {
                prompt_parts.push(v.to_string());
            }
            i += 1;
        } else if a == "--resume" {
            if i + 1 >= args.len() {
                return Err("--resume requires a session id".to_string());
            }
            resume_id = Some(args[i + 1].clone());
            i += 2;
        } else if let Some(v) = a.strip_prefix("--resume=") {
            if v.is_empty() {
                return Err("--resume requires a session id".to_string());
            }
            resume_id = Some(v.to_string());
            i += 1;
        } else if a == "--mode" {
            if i + 1 >= args.len() {
                return Err(
                    "--mode requires auto|default|acceptEdits|plan|dontAsk|bypassPermissions"
                        .to_string(),
                );
            }
            mode =
                Some(Mode::from_name(&args[i + 1]).ok_or_else(|| {
                    format!("unknown --mode \"{}\" (see rem --help)", args[i + 1])
                })?);
            i += 2;
        } else if let Some(v) = a.strip_prefix("--mode=") {
            mode = Some(
                Mode::from_name(v)
                    .ok_or_else(|| format!("unknown --mode \"{v}\" (see rem --help)"))?,
            );
            i += 1;
        } else if a == "--allow-all" {
            allow_all = true;
            i += 1;
        } else if a == "--output" {
            if i + 1 >= args.len() {
                return Err("--output requires text|json|jsonl".to_string());
            }
            output = OutputFormat::parse(&args[i + 1])
                .ok_or_else(|| format!("unknown --output \"{}\" (see rem --help)", args[i + 1]))?;
            i += 2;
        } else if let Some(v) = a.strip_prefix("--output=") {
            output = OutputFormat::parse(v)
                .ok_or_else(|| format!("unknown --output \"{v}\" (see rem --help)"))?;
            i += 1;
        } else if a.starts_with('-') {
            return Err(format!("unknown flag {a} (see rem --help)"));
        } else {
            prompt_parts.push(args[i].clone());
            i += 1;
        }
    }
    if print_mode {
        let prompt = match prompt_parts.is_empty() {
            true => None,
            false => Some(prompt_parts.join(" ")),
        };
        return Ok(Cli::Print {
            prompt,
            resume_id,
            mode,
            allow_all,
            output,
        });
    }
    if mode.is_some() || allow_all || output != OutputFormat::Text {
        return Err("--mode/--allow-all/--output require -p/--print (see rem --help)".to_string());
    }
    if !prompt_parts.is_empty() {
        return Err(format!(
            "unexpected argument \"{}\" (did you mean rem -p \"{}\"? see rem --help)",
            prompt_parts[0],
            prompt_parts.join(" ")
        ));
    }
    Ok(Cli::Run { resume_id })
}

#[tokio::main]
async fn main() {
    // Flags that never touch config, db, or the TUI.
    let args: Vec<String> = std::env::args().collect();
    let cli: Cli = match parse_cli(&args) {
        Ok(Cli::Help) => {
            println!("{HELP}");
            return;
        }
        Ok(Cli::Version) => {
            println!("rem {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Ok(c) => c,
        Err(e) => {
            eprintln!("rem: {e}");
            std::process::exit(2);
        }
    };

    // Headless print mode: no TUI, stdout stays pipeable.
    if let Cli::Print {
        prompt,
        resume_id,
        mode,
        allow_all,
        output,
    } = cli
    {
        let code = headless::run_print(headless::PrintOpts {
            prompt,
            resume_id,
            mode,
            allow_all,
            output,
        })
        .await;
        std::process::exit(code);
    }
    let Cli::Run { resume_id } = cli else {
        unreachable!("help/version/print handled above");
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

    // MCP servers (warn-and-continue: bad entries log, session proceeds).
    let mcp = crate::mcp::load();
    for w in &mcp.warnings {
        eprintln!("rem: {w}");
    }

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

    // Build the agent first so the shared tool registry exists, then
    // connect MCP servers onto it. Warn-and-continue: failures log,
    // the session proceeds on remainder + built-ins. Held connections
    // live in the agent's shared MCP records (process lifetime).
    let mut agent = match RigAgent::new(&cfg, approval_tx, project_root) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("rem: {e}");
            std::process::exit(1);
        }
    };
    let report =
        crate::mcp::connect_all(&mcp, &agent.tool_server_handle(), agent.mcp_state()).await;
    // Startup auto-login display: printable auth URLs for every
    // needs-auth server (SSH/headless copy-paste contract).
    for hint in
        crate::mcp::needs_auth_login_hints(&crate::mcp::needs_auth_servers(agent.mcp_state()))
    {
        eprintln!("rem: {hint}");
    }
    for w in &report.warnings {
        // Config-load warnings already printed above; connect_all
        // carries them through, so dedupe by skipping known ones.
        if !mcp.warnings.contains(w) {
            eprintln!("rem: {w}");
        }
    }
    let section = crate::mcp::preamble_section(&report.inventory);
    if let Err(e) = agent.rebuild_preamble_with_mcp_section(&section) {
        eprintln!("rem: mcp preamble rebuild failed: {e}");
    }

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
    fn bare_positionals_require_print_hint() {
        let err = parse_cli(&argv(&["exec", "do stuff"])).unwrap_err();
        assert!(err.contains("-p"), "got: {err}");
    }

    #[test]
    fn print_captures_prompt_positionals() {
        assert_eq!(
            parse_cli(&argv(&["-p", "do stuff"])),
            Ok(Cli::Print {
                prompt: Some("do stuff".to_string()),
                resume_id: None,
                mode: None,
                allow_all: false,
                output: OutputFormat::Text,
            })
        );
        assert_eq!(
            parse_cli(&argv(&["--print", "do", "stuff"])),
            Ok(Cli::Print {
                prompt: Some("do stuff".to_string()),
                resume_id: None,
                mode: None,
                allow_all: false,
                output: OutputFormat::Text,
            })
        );
    }

    #[test]
    fn print_without_prompt_is_allowed_for_stdin() {
        assert_eq!(
            parse_cli(&argv(&["-p"])),
            Ok(Cli::Print {
                prompt: None,
                resume_id: None,
                mode: None,
                allow_all: false,
                output: OutputFormat::Text,
            })
        );
    }

    #[test]
    fn print_parses_mode_allow_all_output_and_resume() {
        assert_eq!(
            parse_cli(&argv(&[
                "-p",
                "hi",
                "--mode",
                "auto",
                "--allow-all",
                "--output",
                "json",
                "--resume",
                "abc"
            ])),
            Ok(Cli::Print {
                prompt: Some("hi".to_string()),
                resume_id: Some("abc".to_string()),
                mode: Some(Mode::Auto),
                allow_all: true,
                output: OutputFormat::Json,
            })
        );
    }

    #[test]
    fn print_rejects_unknown_mode_and_output() {
        assert!(parse_cli(&argv(&["-p", "hi", "--mode", "turbo"])).is_err());
        assert!(parse_cli(&argv(&["-p", "hi", "--output", "yaml"])).is_err());
    }

    #[test]
    fn headless_flags_require_print() {
        let err = parse_cli(&argv(&["--mode", "auto"])).unwrap_err();
        assert!(err.contains("-p"), "got: {err}");
        let err = parse_cli(&argv(&["--allow-all"])).unwrap_err();
        assert!(err.contains("-p"), "got: {err}");
        let err = parse_cli(&argv(&["--output", "json"])).unwrap_err();
        assert!(err.contains("-p"), "got: {err}");
    }
}
