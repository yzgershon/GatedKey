//! Executing a binding's action.
//!
//! Everything here runs on a worker thread, never on the hook callback. Launching
//! a process can take tens of milliseconds and the hook has a hard deadline.

use std::os::windows::process::CommandExt;
use std::process::Command;

use windows::core::HSTRING;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::config::Action;

/// Do not give the spawned process a console window, and do not make it a child
/// that dies with us: a launched app should outlive GatedKey.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;

pub fn execute(action: &Action) -> Result<(), String> {
    match action {
        Action::LaunchApp { path, args } if args.trim().is_empty() => {
            // ShellExecute handles .exe, .lnk, .bat and anything else with a
            // registered handler. That matters because most things a user thinks
            // of as "an app" are Start Menu shortcuts, and Command::new cannot
            // run a .lnk at all.
            shell_open(path)
        }
        Action::LaunchApp { path, args } => {
            let mut cmd = Command::new(path);
            cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
            for arg in split_args(args) {
                cmd.arg(arg);
            }
            if let Some(dir) = std::path::Path::new(path).parent() {
                if dir.is_dir() {
                    cmd.current_dir(dir);
                }
            }
            cmd.spawn()
                .map(|_| ())
                .map_err(|e| format!("could not launch {path}: {e}"))
        }
        Action::OpenUrl { url } => {
            // Refuse anything that is not plainly a web URL. Without this, a
            // config file becomes a way to run arbitrary schemes.
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err("only http and https URLs are allowed".into());
            }
            shell_open(url)
        }
        Action::OpenFolder { path } => shell_open(path),
        Action::RunCommand { command } => Command::new("cmd")
            .args(["/C", command])
            .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("could not run command: {e}")),
    }
}

fn shell_open(target: &str) -> Result<(), String> {
    unsafe {
        let op = HSTRING::from("open");
        let file = HSTRING::from(target);
        let result = ShellExecuteW(None, &op, &file, None, None, SW_SHOWNORMAL);
        // ShellExecuteW returns a fake HINSTANCE; anything over 32 means success.
        if result.0 as usize > 32 {
            Ok(())
        } else {
            Err(format!("could not open {target}"))
        }
    }
}

/// Split a command-line argument string, honouring double quotes.
fn split_args(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for c in raw.chars() {
        match c {
            '"' => in_quotes = !in_quotes,
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_plain_arguments() {
        assert_eq!(split_args("one two three"), vec!["one", "two", "three"]);
    }

    #[test]
    fn keeps_quoted_paths_together() {
        assert_eq!(
            split_args(r#"--file "C:\Program Files\thing.txt" --flag"#),
            vec![r"--file", r"C:\Program Files\thing.txt", "--flag"]
        );
    }

    #[test]
    fn empty_string_yields_no_arguments() {
        assert!(split_args("").is_empty());
        assert!(split_args("   ").is_empty());
    }

    #[test]
    fn non_web_urls_are_refused() {
        let err = execute(&Action::OpenUrl {
            url: "file:///C:/Windows".into(),
        })
        .unwrap_err();
        assert!(err.contains("http"));
    }
}
