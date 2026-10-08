//! Headless `-p` run path (P0-#2, grilled decisions).
//!
//! - Only `-p`/`--print` (no `exec` subcommand).
//! - Deny-by-default: no approver attached, `Confirm` fails closed to skip.
//!   `--allow-all` attaches an auto-approver (`Confirm` -> `Approve`);
//!   `Deny` (annihilators) still blocks — the safety floor holds.
//! - Text default on stdout (pipeable); `--output json|jsonl` event stream.
//! - Stdin (piped) prepended + `-p` appended; either alone is allowed.
//! - Single `--mode <name>` flag only; persisted sessions + `--resume`.

use crate::{
    agent::{AgentLoop, RigAgent, ToolEvent},
    config::Config,
    modes::Mode,
    permissions::ApprovalDecision,
};

/// Stdout shape for `rem -p`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
    Jsonl,
}

impl OutputFormat {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_lowercase().as_str() {
            "text" => Some(Self::Text),
            "json" => Some(Self::Json),
            "jsonl" => Some(Self::Jsonl),
            _ => None,
        }
    }
}

/// Parsed `rem -p` options (CLI layer fills this; runner executes it).
pub struct PrintOpts {
    pub prompt: Option<String>,
    pub resume_id: Option<String>,
    pub mode: Option<Mode>,
    pub allow_all: bool,
    pub output: OutputFormat,
}

/// Read piped stdin. Empty when stdin is a TTY (no pipe), so headless never
/// blocks waiting for interactive input.
pub fn read_stdin_piped() -> String {
    use std::io::{IsTerminal, Read};
    if std::io::stdin().is_terminal() {
        return String::new();
    }
    let mut buf = String::new();
    match std::io::stdin().read_to_string(&mut buf) {
        Ok(_) => buf,
        Err(_) => String::new(),
    }
}

/// Combine piped stdin + `-p` prompt. Stdin first (context), flag second
/// (instruction); either alone is allowed. Empty when both are empty.
pub fn combine_prompt(stdin_text: &str, flag_prompt: Option<&str>) -> String {
    let mut parts = Vec::new();
    if !stdin_text.trim().is_empty() {
        parts.push(stdin_text.trim().to_string());
    }
    if let Some(p) = flag_prompt
        && !p.trim().is_empty()
    {
        parts.push(p.trim().to_string());
    }
    parts.join("\n\n")
}

/// One tool event as a JSON object for `--output json|jsonl`.
pub fn tool_json(ev: &ToolEvent) -> serde_json::Value {
    serde_json::json!({
        "type": "tool",
        "name": ev.name,
        "args": ev.args,
        "ok": ev.ok,
        "preview": ev.arg_preview(),
        "summary": ev.summary,
        "output": ev.output,
    })
}

/// Full turn as a JSON array: tool events + terminal result/error object.
pub fn json_doc(events: &[ToolEvent], result: &Result<String, String>) -> serde_json::Value {
    let mut arr: Vec<serde_json::Value> = events.iter().map(tool_json).collect();
    match result {
        Ok(text) => arr.push(serde_json::json!({"type": "result", "text": text})),
        Err(e) => arr.push(serde_json::json!({"type": "error", "message": e})),
    }
    serde_json::Value::Array(arr)
}

/// Render `--output json` (single pretty document) / `--output jsonl`
/// (one object per line).
pub fn render_json(events: &[ToolEvent], result: &Result<String, String>) -> String {
    serde_json::to_string_pretty(&json_doc(events, result)).unwrap_or_default()
}

pub fn render_jsonl(events: &[ToolEvent], result: &Result<String, String>) -> String {
    let mut lines = Vec::new();
    for ev in events {
        lines.push(serde_json::to_string(&tool_json(ev)).unwrap_or_default());
    }
    let tail = match result {
        Ok(text) => serde_json::json!({"type": "result", "text": text}),
        Err(e) => serde_json::json!({"type": "error", "message": e}),
    };
    lines.push(serde_json::to_string(&tail).unwrap_or_default());
    lines.join("\n")
}

/// Execute one headless turn. Returns the process exit code; all user-facing
/// messages go to stderr except the turn payload on stdout (pipeable).
pub async fn run_print(opts: PrintOpts) -> i32 {
    // Fail fast on empty prompt BEFORE touching config/db so usage errors
    // neither require config nor pollute the sessions table with rows.
    let stdin_text = read_stdin_piped();
    let combined_early = combine_prompt(&stdin_text, opts.prompt.as_deref());
    if combined_early.trim().is_empty() {
        eprintln!(
            "rem: no prompt: pass -p \"<prompt>\" and/or pipe stdin (echo ... | rem -p \"...\")"
        );
        return 2;
    }

    let cfg = match Config::from_file() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("rem: {e}");
            eprintln!(
                "hint: create ~/.config/rem/config.toml with active_provider + [providers.*] sections (see config.example.toml)"
            );
            return 1;
        }
    };

    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let project_root = cwd.canonicalize().unwrap_or(cwd);
    let project_root_str = project_root.to_string_lossy().to_string();

    let conn = match crate::sessions::open() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("rem: {e}");
            return 1;
        }
    };

    // New session or resume; resume of an unknown id is fatal (mirrors TUI).
    let is_resume = opts.resume_id.is_some();
    let mut session = match opts.resume_id {
        Some(id) => match crate::sessions::get_session(&conn, &id) {
            Ok(Some(s)) => s,
            Ok(None) => {
                eprintln!("rem: no session with id {id}");
                return 1;
            }
            Err(e) => {
                eprintln!("rem: {e}");
                return 1;
            }
        },
        None => {
            let tag = match cfg.active() {
                Ok(r) => format!("{}:{}", r.name, r.model),
                Err(e) => {
                    eprintln!("rem: {e}");
                    return 1;
                }
            };
            match crate::sessions::create_session(&conn, &project_root_str, &tag) {
                Ok(s) => {
                    if let Err(e) = crate::sessions::update_title(&conn, &s.id, "untitled") {
                        eprintln!("rem: {e}");
                        return 1;
                    }
                    crate::sessions::get_session(&conn, &s.id)
                        .ok()
                        .flatten()
                        .unwrap_or(s)
                }
                Err(e) => {
                    eprintln!("rem: {e}");
                    return 1;
                }
            }
        }
    };
    if session.title != "untitled" && !is_resume {
        session.title = "untitled".to_string();
    }
    let session_id = session.id.clone();

    let (approval_tx, approval_rx) = crate::permissions::approval_channel();
    let agent = match RigAgent::new(&cfg, approval_tx, project_root.clone()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("rem: {e}");
            return 1;
        }
    };

    // Headless mode flag (grilled: single `--mode`, default `manual`).
    if let Some(m) = opts.mode
        && let Ok(mut guard) = agent.mode_handle().lock()
    {
        *guard = m;
    }
    // Headless has no TUI loop to observe completion: `background:true`
    // degrades to inline (same result, no polling) instead of detaching.
    agent.set_background_enabled(false);

    let agent = std::sync::Arc::new(agent);

    // Resume: switch to the stored provider tag + import history (TUI parity).
    // Unknown tags warn on stderr but keep history on the current client.
    if is_resume {
        if !session.model.is_empty() && session.model != agent.model_name() {
            match agent.switch_model_sync(&session.model) {
                Ok(_) => {}
                Err(e) => {
                    eprintln!(
                        "rem: resume: stored model {} unavailable ({e}), kept {}",
                        session.model,
                        agent.model_name()
                    );
                }
            }
        }
        if let Err(e) = agent.import_messages_json(&session.messages_json).await {
            eprintln!("rem: failed to restore session history: {e}");
            return 1;
        }
    }

    // Prompt already validated above; reuse it for the turn.
    let combined = combined_early;

    // `@path` + `/skill:name` expansion (same semantics as the TUI submit
    // path; failures are fatal headless instead of a transcript notice).
    // Skill bodies expand first so `@refs` inside skill tasks also resolve.
    let mut display = combined.clone();
    let mut prompt_text = combined.clone();
    match crate::tui::expand_skill_invokes(&prompt_text, &agent) {
        Ok(Some((short, expanded))) => {
            display = short;
            prompt_text = expanded;
        }
        Ok(None) => {}
        Err(e) => {
            eprintln!("rem: skill: {e}");
            return 1;
        }
    }
    match crate::tui::expand_at_refs(&prompt_text, &project_root) {
        Ok(e) if e != prompt_text => prompt_text = e,
        Ok(_) => {}
        Err(e) => {
            eprintln!("rem: @ mention: {e}");
            return 1;
        }
    }

    // Approver wiring (grilled deny-by-default):
    // - default: drop `rx` so `Confirm` fails closed to skip in-run.
    // - `--allow-all`: background task auto-approves every `Confirm`;
    //   `Deny` verdicts never reach the channel, so annihilators stay blocked.
    if opts.allow_all {
        let mut rx = approval_rx;
        tokio::spawn(async move {
            while let Some(req) = rx.recv().await {
                let _ = req.reply.send(ApprovalDecision::Approve {
                    comment: String::new(),
                });
            }
        });
    } else {
        drop(approval_rx);
    }

    let result = agent.chat(&prompt_text).await;
    let events = agent.last_tool_events();
    let code = match &result {
        Ok(_) => 0,
        Err(_) => 1,
    };

    match opts.output {
        OutputFormat::Text => {
            for ev in &events {
                eprintln!("● {} {} — {}", ev.name, ev.arg_preview(), ev.summary);
            }
            match &result {
                Ok(text) => println!("{text}"),
                Err(e) => eprintln!("rem: {e}"),
            }
        }
        OutputFormat::Json => {
            println!("{}", render_json(&events, &result));
            if let Err(e) = &result {
                eprintln!("rem: {e}");
            }
        }
        OutputFormat::Jsonl => {
            println!("{}", render_jsonl(&events, &result));
            if let Err(e) = &result {
                eprintln!("rem: {e}");
            }
        }
    }

    // Persist on success only (TUI parity); stdout stays pipeable so the
    // resume hint goes to stderr.
    if result.is_ok() {
        match agent.export_messages_json().await {
            Ok(msgs) => {
                let _ = crate::sessions::save_messages(&conn, &session_id, &msgs);
                let _ = crate::sessions::touch(&conn, &session_id);
                if session.title.is_empty() || session.title == "untitled" {
                    let t: String = display.chars().take(40).collect();
                    if !t.trim().is_empty() {
                        let _ = crate::sessions::update_title(&conn, &session_id, &t);
                    }
                }
            }
            Err(e) => eprintln!("rem: save failed: {e}"),
        }
        eprintln!("Session saved. Resume with: rem --resume {session_id}");
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdin_and_flag_combine_with_stdin_first() {
        assert_eq!(combine_prompt("", None), "");
        assert_eq!(combine_prompt("", Some("do x")), "do x");
        assert_eq!(combine_prompt("ctx", None), "ctx");
        assert_eq!(combine_prompt("ctx", Some("do x")), "ctx\n\ndo x");
        assert_eq!(combine_prompt("  ", Some("  ")), "");
    }

    #[test]
    fn output_format_parses_known_shapes() {
        assert_eq!(OutputFormat::parse("text"), Some(OutputFormat::Text));
        assert_eq!(OutputFormat::parse("json"), Some(OutputFormat::Json));
        assert_eq!(OutputFormat::parse("jsonl"), Some(OutputFormat::Jsonl));
        assert_eq!(OutputFormat::parse("JSON"), Some(OutputFormat::Json));
        assert_eq!(OutputFormat::parse("yaml"), None);
    }

    #[test]
    fn json_doc_carries_tools_plus_result() {
        let ev = ToolEvent {
            name: "read".to_string(),
            args: serde_json::json!({"path": "a.rs"}),
            ok: true,
            summary: "ok".to_string(),
            output: "body".to_string(),
        };
        let doc = json_doc(&[ev], &Ok("done".to_string()));
        assert_eq!(doc.as_array().unwrap().len(), 2);
        assert_eq!(doc[0]["type"], "tool");
        assert_eq!(doc[0]["name"], "read");
        assert_eq!(doc[1]["type"], "result");
        assert_eq!(doc[1]["text"], "done");
    }

    #[test]
    fn json_doc_carries_error_tail() {
        let doc = json_doc(&[], &Err("boom".to_string()));
        assert_eq!(doc.as_array().unwrap().len(), 1);
        assert_eq!(doc[0]["type"], "error");
    }

    #[test]
    fn jsonl_renders_one_object_per_line() {
        let ev = ToolEvent {
            name: "bash".to_string(),
            args: serde_json::json!({"command": "ls"}),
            ok: true,
            summary: "ok".to_string(),
            output: "x".to_string(),
        };
        let out = render_jsonl(&[ev], &Ok("done".to_string()));
        assert_eq!(out.lines().count(), 2);
        for line in out.lines() {
            assert!(serde_json::from_str::<serde_json::Value>(line).is_ok());
        }
    }
}
