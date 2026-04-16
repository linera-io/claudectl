use super::run_osascript;
use crate::session::ClaudeSession;

/// Find the best matching Ghostty terminal for a session.
/// Ghostty's AppleScript API exposes: id, name, working directory (no tty/pid).
/// Strategy: match by CWD, disambiguate by session name in terminal title.
fn find_terminal_script(session: &ClaudeSession) -> String {
    let cwd = session.cwd.replace('"', "\\\"");
    let session_name = session.session_name.replace('"', "\\\"");

    // If we have a session name, try to match it against the terminal title first.
    // Claude Code sets the terminal title to "<spinner> <task_description>" which
    // often contains the session name (from --name or --resume flags).
    if session_name.is_empty() {
        // No session name — match by CWD only, take first match
        format!(
            r#"
            set matches to every terminal whose working directory contains "{cwd}"
            if (count of matches) = 0 then error "No Ghostty terminal found for {cwd}"
            set t to item 1 of matches
            "#,
        )
    } else {
        // Try CWD + name match first, fall back to CWD-only
        format!(
            r#"
            set matches to every terminal whose working directory contains "{cwd}"
            if (count of matches) = 0 then error "No Ghostty terminal found for {cwd}"

            -- Disambiguate: find the terminal whose title contains our session name
            set t to item 1 of matches
            repeat with candidate in matches
                if name of candidate contains "{session_name}" then
                    set t to candidate
                    exit repeat
                end if
            end repeat
            "#,
        )
    }
}

pub fn switch(session: &ClaudeSession) -> Result<(), String> {
    let find = find_terminal_script(session);

    let script = format!(
        r#"
        tell application "Ghostty"
            {find}
            focus t
            activate
        end tell
        "#,
    );

    run_osascript(&script)
}

pub fn send_input(session: &ClaudeSession, text: &str) -> Result<(), String> {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    let find = find_terminal_script(session);

    let script = format!(
        r#"
        tell application "Ghostty"
            {find}
            input text "{escaped}" to t
        end tell
        "#,
    );
    run_osascript(&script)
}

pub fn approve(session: &ClaudeSession) -> Result<(), String> {
    let find = find_terminal_script(session);

    let script = format!(
        r#"
        tell application "Ghostty"
            {find}
            send key "enter" to t
        end tell
        "#,
    );
    run_osascript(&script)
}
