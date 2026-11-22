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
    match env_passphrase() {
        Some(p) => check_new(p.expose_secret(), None).map(|()| p),
        None => new_from(prompt, profile),
    }
}

/// Asks twice through `ask` (the tty prompt in production) and checks the answers.
fn new_from(
    mut ask: impl FnMut(&str) -> io::Result<String>,
    profile: &ProfileName,
) -> Result<SecretString> {
    let first = ask(&format!("New passphrase for \"{profile}\": ")).map_err(no_terminal)?;
    let second = ask("Repeat the passphrase: ").map_err(no_terminal)?;
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
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")?;
    ask_yes(question, BufReader::new(&tty), &tty)
}

fn ask_yes(question: &str, mut input: impl BufRead, mut output: impl Write) -> Result<bool> {
    write!(output, "{question} [y/N] ")?;
    output.flush()?;
    let mut line = String::new();
    input.read_line(&mut line)?;
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

    fn answers(list: &[&'static str]) -> impl FnMut(&str) -> io::Result<String> {
        let mut it = list.iter();
        move |_| Ok(it.next().expect("no more answers").to_string())
    }

    fn enxio() -> io::Error {
        io::Error::from_raw_os_error(ENXIO)
    }

    #[test]
    fn missing_terminal_gets_a_hint() {
        for e in [enxio(), io::ErrorKind::NotFound.into()] {
            assert!(
                no_terminal(e)
                    .to_string()
                    .contains("set DUCKPOND_PASSPHRASE")
            );
        }
        let other = no_terminal(io::ErrorKind::PermissionDenied.into()).to_string();
        assert!(!other.contains("DUCKPOND_PASSPHRASE"), "{other}");
    }

    #[test]
    fn new_passphrase_asks_twice_and_checks() {
        let p = new_from(answers(&["password1", "password1"]), &"P".parse().unwrap());
        assert_eq!(p.unwrap().expose_secret(), "password1");
        let e = new_from(answers(&["password1", "password2"]), &"P".parse().unwrap());
        assert!(e.unwrap_err().to_string().contains("do not match"));
        let e = new_from(|_| Err(enxio()), &"P".parse().unwrap());
        assert!(e.unwrap_err().to_string().contains("no terminal"));
    }

    #[test]
    fn new_passphrase_prompts_name_the_profile() {
        let mut seen = Vec::new();
        let ask = |q: &str| {
            seen.push(q.to_string());
            Ok("password1".to_string())
        };
        new_from(ask, &"My Prof".parse().unwrap()).unwrap();
        assert_eq!(
            seen,
            [
                "New passphrase for \"My Prof\": ",
                "Repeat the passphrase: "
            ]
        );
    }

    #[test]
    fn confirm_accepts_only_yes() {
        for (line, want) in [
            ("y\n", true),
            ("YES\n", true),
            (" yes \n", true),
            ("\n", false),
            ("no\n", false),
            ("", false),
        ] {
            let mut out = Vec::new();
            assert_eq!(
                ask_yes("Delete?", line.as_bytes(), &mut out).unwrap(),
                want,
                "{line:?}"
            );
            assert_eq!(out, b"Delete? [y/N] ");
        }
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
