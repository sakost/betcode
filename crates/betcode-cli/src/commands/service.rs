//! Service command parsing and interception.
//!
//! Service commands are handled locally by betcode rather than being
//! forwarded to Claude Code. They include /cd, /pwd, /exit, /exit-daemon,
//! /reload-remote, and !bash shortcuts.

/// A parsed service command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceCommand {
    Cd { path: String },
    Pwd,
    Exit,
    ExitDaemon,
    ReloadRemote,
    Bash { cmd: String },
}

/// Known service command names (without the leading `/`).
const SERVICE_COMMANDS: &[&str] = &["cd", "pwd", "exit", "exit-daemon", "reload-remote"];

/// Returns true if the input is a service command (handled locally by betcode).
pub fn is_service_command(input: &str) -> bool {
    let trimmed = input.trim();

    // !<anything> is always a bash command
    if trimmed.starts_with('!') && trimmed.len() > 1 {
        return true;
    }

    // /<command> must match a known service command
    if let Some(rest) = trimmed.strip_prefix('/') {
        let cmd_name = rest.split_whitespace().next().unwrap_or("");
        return SERVICE_COMMANDS.contains(&cmd_name);
    }

    false
}

/// Parses input into a typed ServiceCommand, returning None if not a service command.
pub fn parse_service_command(input: &str) -> Option<ServiceCommand> {
    let trimmed = input.trim();

    // !<bash command>
    if let Some(cmd) = trimmed.strip_prefix('!') {
        let cmd = cmd.trim();
        if cmd.is_empty() {
            return None;
        }
        return Some(ServiceCommand::Bash {
            cmd: cmd.to_string(),
        });
    }

    // /<service command>
    let rest = trimmed.strip_prefix('/')?;
    let mut parts = rest.splitn(2, char::is_whitespace);
    let cmd_name = parts.next().unwrap_or("");
    let args = parts.next().unwrap_or("").trim();

    match cmd_name {
        "cd" => Some(ServiceCommand::Cd {
            path: if args.is_empty() {
                "~".to_string()
            } else {
                args.to_string()
            },
        }),
        "pwd" => Some(ServiceCommand::Pwd),
        "exit" => Some(ServiceCommand::Exit),
        "exit-daemon" => Some(ServiceCommand::ExitDaemon),
        "reload-remote" => Some(ServiceCommand::ReloadRemote),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_service_command() {
        assert!(is_service_command("/cd /tmp"));
        assert!(is_service_command("/pwd"));
        assert!(is_service_command("/exit"));
        assert!(is_service_command("/exit-daemon"));
        assert!(is_service_command("!ls -la"));
        assert!(!is_service_command("hello world"));
        assert!(!is_service_command("/help")); // Claude Code command, not service
    }

    #[test]
    fn test_parse_service_command() {
        let cmd = parse_service_command("/cd /tmp").unwrap();
        assert_eq!(
            cmd,
            ServiceCommand::Cd {
                path: "/tmp".to_string()
            }
        );

        let cmd = parse_service_command("/pwd").unwrap();
        assert_eq!(cmd, ServiceCommand::Pwd);

        let cmd = parse_service_command("/exit").unwrap();
        assert_eq!(cmd, ServiceCommand::Exit);

        let cmd = parse_service_command("!echo hello").unwrap();
        assert_eq!(
            cmd,
            ServiceCommand::Bash {
                cmd: "echo hello".to_string()
            }
        );
    }

    #[test]
    fn test_parse_exit_daemon() {
        let cmd = parse_service_command("/exit-daemon").unwrap();
        assert_eq!(cmd, ServiceCommand::ExitDaemon);
    }

    #[test]
    fn test_parse_reload_remote() {
        let cmd = parse_service_command("/reload-remote").unwrap();
        assert_eq!(cmd, ServiceCommand::ReloadRemote);
    }

    #[test]
    fn test_cd_no_args_defaults_to_home() {
        let cmd = parse_service_command("/cd").unwrap();
        assert_eq!(
            cmd,
            ServiceCommand::Cd {
                path: "~".to_string()
            }
        );
    }

    #[test]
    fn test_non_service_command_returns_none() {
        assert!(parse_service_command("/help").is_none());
        assert!(parse_service_command("/clear").is_none());
        assert!(parse_service_command("hello world").is_none());
    }

    #[test]
    fn test_empty_bash_returns_none() {
        assert!(parse_service_command("!").is_none());
        assert!(parse_service_command("! ").is_none());
    }

    #[test]
    fn test_whitespace_handling() {
        let cmd = parse_service_command("  /cd   /tmp  ").unwrap();
        assert_eq!(
            cmd,
            ServiceCommand::Cd {
                path: "/tmp".to_string()
            }
        );

        let cmd = parse_service_command("  !echo hello  ").unwrap();
        assert_eq!(
            cmd,
            ServiceCommand::Bash {
                cmd: "echo hello".to_string()
            }
        );
    }
}
