//! TTY / 管道审批提示（对齐 Server 非交互回退语义）。

use std::io::{self, IsTerminal, Write};

use crabmate_tui_core::{ApprovalDecision, ApprovalGate, CommandApprovalRequest, TermError};

/// 在 stderr 提示，从 stdin 读一行决策。
#[derive(Debug, Default)]
pub struct TtyApprovalGate;

impl TtyApprovalGate {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl ApprovalGate for TtyApprovalGate {
    fn decide(&mut self, req: &CommandApprovalRequest) -> Result<ApprovalDecision, TermError> {
        print_prompt(req)?;
        read_decision()
    }
}

fn print_prompt(req: &CommandApprovalRequest) -> Result<(), TermError> {
    let mut err = io::stderr().lock();
    let cmd = req.command.trim();
    let args = req.args.trim();
    let preview = if args.is_empty() {
        cmd.to_string()
    } else {
        format!("{cmd} {args}")
    };
    writeln!(err, "command approval required: {preview}")
        .map_err(|e| TermError::Message(format!("stderr write failed: {e}")))?;
    if let Some(key) = req.allowlist_key.as_deref().filter(|s| !s.is_empty()) {
        writeln!(err, "  allowlist_key={key}")
            .map_err(|e| TermError::Message(format!("stderr write failed: {e}")))?;
    }
    if io::stdin().is_terminal() && io::stderr().is_terminal() {
        writeln!(err, "  d=deny (Enter)  o=once  a=always")
            .map_err(|e| TermError::Message(format!("stderr write failed: {e}")))?;
    } else {
        writeln!(
            err,
            "  non-interactive: y/once, a/always, n/deny (Enter or empty=deny)"
        )
        .map_err(|e| TermError::Message(format!("stderr write failed: {e}")))?;
    }
    err.flush()
        .map_err(|e| TermError::Message(format!("stderr flush failed: {e}")))?;
    Ok(())
}

fn read_decision() -> Result<ApprovalDecision, TermError> {
    let mut line = String::new();
    match io::stdin().read_line(&mut line) {
        Ok(0) => Ok(ApprovalDecision::Deny),
        Ok(_) => Ok(parse_decision_line(&line)),
        Err(e) if e.kind() == io::ErrorKind::Interrupted => Err(TermError::Interrupted),
        Err(e) => Err(TermError::Message(format!("read approval decision: {e}"))),
    }
}

fn parse_decision_line(line: &str) -> ApprovalDecision {
    let t = line.trim().to_ascii_lowercase();
    match t.as_str() {
        "" | "d" | "n" | "no" | "deny" => ApprovalDecision::Deny,
        "a" | "always" | "allow_always" => ApprovalDecision::AllowAlways,
        "o" | "y" | "yes" | "once" | "allow_once" => ApprovalDecision::AllowOnce,
        _ => ApprovalDecision::Deny,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_decision_aliases() {
        assert_eq!(parse_decision_line("y"), ApprovalDecision::AllowOnce);
        assert_eq!(parse_decision_line("ONCE"), ApprovalDecision::AllowOnce);
        assert_eq!(parse_decision_line("a"), ApprovalDecision::AllowAlways);
        assert_eq!(parse_decision_line(""), ApprovalDecision::Deny);
        assert_eq!(parse_decision_line("nope"), ApprovalDecision::Deny);
    }
}
