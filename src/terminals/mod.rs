#[cfg(target_os = "macos")]
mod apple;
#[cfg(target_os = "macos")]
mod ghostty;
#[cfg(target_os = "macos")]
mod iterm2;
mod kitty;
mod tmux;
#[cfg(target_os = "macos")]
mod warp;
mod wezterm;

use crate::session::ClaudeSession;

pub enum Terminal {
    Ghostty,
    Warp,
    ITerm2,
    Kitty,
    WezTerm,
    Apple,
    Tmux,
    Unknown(String),
}

fn terminal_name(t: &Terminal) -> &str {
    match t {
        Terminal::Ghostty => "Ghostty",
        Terminal::Warp => "Warp",
        Terminal::ITerm2 => "iTerm2",
        Terminal::Kitty => "Kitty",
        Terminal::WezTerm => "WezTerm",
        Terminal::Apple => "Apple Terminal",
        Terminal::Tmux => "tmux",
        Terminal::Unknown(name) => name,
    }
}

pub(crate) fn build_claude_args(prompt: Option<&str>, resume: Option<&str>) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(resume_id) = resume {
        args.push("--resume".to_string());
        args.push(resume_id.to_string());
    }
    if let Some(prompt_text) = prompt {
        args.push("-p".to_string());
        args.push(prompt_text.to_string());
    }
    args
}

pub(crate) fn shell_escape(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\"'\"'"))
}

pub fn detect_terminal() -> Terminal {
    if std::env::var("TMUX").is_ok() {
        return Terminal::Tmux;
    }

    match std::env::var("TERM_PROGRAM").as_deref() {
        Ok("ghostty") => Terminal::Ghostty,
        Ok("WarpTerminal") => Terminal::Warp,
        Ok("iTerm.app") => Terminal::ITerm2,
        Ok("kitty") => Terminal::Kitty,
        Ok("WezTerm") => Terminal::WezTerm,
        Ok("Apple_Terminal") => Terminal::Apple,
        Ok(other) => Terminal::Unknown(other.to_string()),
        Err(_) => Terminal::Unknown("unknown".to_string()),
    }
}

pub fn can_launch_session() -> bool {
    matches!(
        detect_terminal(),
        Terminal::Kitty | Terminal::Tmux | Terminal::WezTerm
    )
}

pub fn launch_session(cwd: &str, prompt: Option<&str>, resume: Option<&str>) -> Result<String, String> {
    let terminal = detect_terminal();
    match terminal {
        Terminal::Kitty => kitty::launch(cwd, prompt, resume),
        Terminal::Tmux => tmux::launch(cwd, prompt, resume),
        Terminal::WezTerm => wezterm::launch(cwd, prompt, resume),
        other => Err(format!(
            "Visible session launch is not supported in {}. Start `claude` manually, or use tmux, Kitty, or WezTerm.",
            terminal_name(&other)
        )),
    }
}

pub fn switch_to_terminal(session: &ClaudeSession) -> Result<(), String> {
    if session.tty.is_empty() {
        return Err("No TTY associated with this session".into());
    }

    let terminal = detect_terminal();
    crate::logger::log(
        "DEBUG",
        &format!(
            "terminal switch: {} (tty={}) via {:?}",
            session.display_name(),
            session.tty,
            terminal_name(&terminal)
        ),
    );

    match terminal {
        Terminal::Kitty => kitty::switch(session),
        Terminal::WezTerm => wezterm::switch(session),
        Terminal::Tmux => tmux::switch(session),
        #[cfg(target_os = "macos")]
        Terminal::Ghostty => ghostty::switch(session),
        #[cfg(target_os = "macos")]
        Terminal::Warp => warp::switch(session),
        #[cfg(target_os = "macos")]
        Terminal::ITerm2 => iterm2::switch(session),
        #[cfg(target_os = "macos")]
        Terminal::Apple => apple::switch(session),
        Terminal::Unknown(name) => Err(format!(
            "Unsupported terminal: {name}. Supported: Ghostty, Warp, iTerm2, Kitty, WezTerm, Terminal.app, tmux"
        )),
        #[cfg(not(target_os = "macos"))]
        _ => Err("Terminal switching not supported on this platform".into()),
    }
}

pub fn send_input(session: &ClaudeSession, text: &str) -> Result<(), String> {
    match detect_terminal() {
        #[cfg(target_os = "macos")]
        Terminal::Ghostty => ghostty::send_input(session, text),
        Terminal::Kitty => kitty::send_input(session, text),
        Terminal::Tmux => tmux::send_input(session, text),
        #[cfg(target_os = "macos")]
        Terminal::Warp => warp::send_input(session, text),
        #[cfg(target_os = "macos")]
        _ => {
            // iTerm2, Apple Terminal, etc: switch + System Events keystroke
            switch_to_terminal(session)?;
            std::thread::sleep(std::time::Duration::from_millis(300));
            let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
            run_osascript(&format!(
                r#"tell application "System Events" to keystroke "{escaped}""#,
            ))
        }
        #[cfg(not(target_os = "macos"))]
        _ => Err("Input injection not supported for this terminal".into()),
    }
}

pub fn approve_session(session: &ClaudeSession) -> Result<(), String> {
    match detect_terminal() {
        #[cfg(target_os = "macos")]
        Terminal::Ghostty => ghostty::approve(session),
        Terminal::Kitty => kitty::approve(session),
        Terminal::Tmux => tmux::send_input(session, "\r"),
        #[cfg(target_os = "macos")]
        Terminal::Warp => warp::approve(session),
        #[cfg(target_os = "macos")]
        _ => {
            // iTerm2, Apple Terminal, etc: switch + press Enter
            switch_to_terminal(session)?;
            std::thread::sleep(std::time::Duration::from_millis(300));
            run_osascript(r#"tell application "System Events" to key code 36"#)
        }
        #[cfg(not(target_os = "macos"))]
        _ => Err("Input injection not supported for this terminal".into()),
    }
}

#[cfg(target_os = "macos")]
pub fn run_osascript(script: &str) -> Result<(), String> {
    let output = std::process::Command::new("osascript")
        .args(["-e", script])
        .output()
        .map_err(|e| format!("Failed to run osascript: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("AppleScript error: {}", stderr.trim()))
    }
}
