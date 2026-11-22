//! All terminal input. Prompts use /dev/tty so they work when stdin or stdout is a pipe.

use std::io::{self, BufRead, BufReader, IsTerminal, Read, Write};

use age::secrecy::{ExposeSecret, SecretString};

use crate::error::{Error, Result};
use crate::name::{ProfileName, VarName};

const ENV: &str = "DUCKPOND_PASSPHRASE";
const MIN_LEN: usize = 8;
/// errno of open("/dev/tty") when the process has no controlling terminal.
const ENXIO: i32 = 6;

fn env_passphrase() -> Option<SecretString> {
    std::env::var(ENV)
        .ok()
        .filter(|s| !s.is_empty())
        .map(SecretString::from)
}

/// Hidden prompt on /dev/tty (`rpassword::prompt_password` opens /dev/tty for both prompt and input).
fn prompt(text: &str) -> io::Result<String> {
    rpassword::prompt_password(text)
}

fn no_terminal(e: io::Error) -> Error {
    if e.kind() == io::ErrorKind::NotFound || e.raw_os_error() == Some(ENXIO) {
        return format!("no terminal to ask for the passphrase; set {ENV}").into();
    }
    e.into()
}

pub fn passphrase(profile: &ProfileName) -> Result<SecretString> {
    if let Some(p) = env_passphrase() {
        return Ok(p);
    }
    let p = prompt(&format!("Passphrase for \"{profile}\": ")).map_err(no_terminal)?;
    Ok(SecretString::from(p))
}

fn check_new(first: &str, second: Option<&str>) -> Result<()> {
    if first.chars().count() < MIN_LEN {
        return Err(format!("the passphrase must have at least {MIN_LEN} characters").into());
    }
    match second {
        Some(s) if s != first => Err("the passphrases do not match".to_string().into()),
        _ => Ok(()),
    }
}

pub fn new_passphrase(profile: &ProfileName) -> Result<SecretString> {
    if let Some(p) = env_passphrase() {
        check_new(p.expose_secret(), None)?;
        return Ok(p);
    }
    let first = prompt(&format!("New passphrase for \"{profile}\": ")).map_err(no_terminal)?;
    let second = prompt("Repeat the passphrase: ").map_err(no_terminal)?;
    check_new(&first, Some(&second))?;
    Ok(SecretString::from(first))
}

/// Exactly one value, all of `reader` minus one trailing newline.
fn from_reader(names: &[VarName], mut reader: impl Read) -> Result<Vec<String>> {
    if names.len() != 1 {
        return Err(
            "stdin is not a terminal: give exactly one NAME to read its value from stdin"
                .to_string()
                .into(),
        );
    }
    let mut text = String::new();
    reader.read_to_string(&mut text)?;
    if text.ends_with('\n') {
        text.pop();
    }
    Ok(vec![text])
}

pub fn values(names: &[VarName]) -> Result<Vec<String>> {
    let stdin = io::stdin();
    if !stdin.is_terminal() {
        return from_reader(names, stdin.lock());
    }
    let ask = |n: &VarName| prompt(&format!("Value for {n}: "));
    Ok(names.iter().map(ask).collect::<io::Result<_>>()?)
}

pub fn confirm(question: &str) -> Result<bool> {
    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")?;
    write!(tty, "{question} [y/N] ")?;
    tty.flush()?;
    let mut line = String::new();
    BufReader::new(&tty).read_line(&mut line)?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: &[&str]) -> Vec<VarName> {
        n.iter().map(|s| s.parse().unwrap()).collect()
    }

    fn read(n: &[&str], input: &str) -> Result<Vec<String>> {
        from_reader(&names(n), input.as_bytes())
    }

    #[test]
    fn strips_exactly_one_trailing_newline() {
        assert_eq!(read(&["A"], "v\n").unwrap(), ["v"]);
        assert_eq!(read(&["A"], "v\n\n").unwrap(), ["v\n"]);
        assert_eq!(read(&["A"], "v").unwrap(), ["v"]);
        assert_eq!(read(&["A"], "a\nb\n").unwrap(), ["a\nb"]);
        assert_eq!(read(&["A"], "").unwrap(), [""]);
        assert_eq!(read(&["A"], "\r\n").unwrap(), ["\r"]);
    }

    #[test]
    fn piped_stdin_needs_exactly_one_name() {
        assert!(read(&["A", "B"], "v").is_err());
        assert!(read(&[], "v").is_err());
    }

    #[test]
    fn invalid_utf8_is_an_error() {
        assert!(from_reader(&names(&["A"]), &b"\xff"[..]).is_err());
    }

    #[test]
    fn new_passphrase_rules() {
        assert!(check_new("1234567", None).is_err());
        assert!(check_new("12345678", None).is_ok());
        assert!(check_new("12345678", Some("12345678")).is_ok());
        assert!(check_new("12345678", Some("12345679")).is_err());
        assert!(
            check_new("ééééééé", None).is_err(),
            "counts characters, not bytes"
        );
    }
}
