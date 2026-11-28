//! Profile choice: completion candidates and the interactive picker.

use std::ffi::OsStr;
use std::fmt::Display;
use std::io::IsTerminal;

use clap_complete::engine::{ArgValueCompleter, CompletionCandidate};
use inquire::InquireError;

use crate::crypto;
use crate::error::{Error, Result};
use crate::name::{ProfileName, VarName};
use crate::store::Store;
use crate::vars::Vars;

/// Profile names for shell completion. Any error gives an empty list: no prompt, no decryption.
/// The shell passes the word with its quoting (`Fresh\ D`, `"Fresh D`). A name has no quote
/// or backslash, so removing them leaves the text the user means, in any shell.
pub fn candidates() -> ArgValueCompleter {
    ArgValueCompleter::new(|current: &OsStr| {
        let typed = current.to_string_lossy().replace(['"', '\'', '\\'], "");
        let names = Store::from_env().and_then(|s| s.list()).unwrap_or_default();
        names
            .iter()
            .filter(|n| n.as_ref().starts_with(&typed))
            .map(|n| CompletionCandidate::new(n.to_string()))
            .collect()
    })
}

/// The picker needs a terminal to read keys and one to draw on (it draws on stderr).
fn interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// The profile given on the command line, else one chosen in the picker.
pub fn profile(store: &Store, given: Option<ProfileName>) -> Result<ProfileName> {
    if let Some(name) = given {
        return Ok(name);
    }
    if !interactive() {
        return Err("missing PROFILE".to_string().into());
    }
    select("Profile", store.list()?, NO_PROFILES)
}

const NO_PROFILES: &str = "no profiles; create one with: duckpond create NAME";

/// The variable given on the command line, else one chosen in the picker.
pub fn var(vars: &Vars, given: Option<VarName>) -> Result<VarName> {
    if let Some(name) = given {
        return Ok(name);
    }
    if !interactive() {
        return Err("missing NAME".to_string().into());
    }
    select("Variable", vars.0.keys().cloned().collect(), NO_VARS)
}

const NO_VARS: &str = "no variables; add one with: duckpond set PROFILE NAME";

/// Fingerprint of a GPG secret key chosen in the menu (`create --gpg` without KEY).
pub fn gpg_key() -> Result<String> {
    if !interactive() {
        return Err("no terminal to pick a GPG key; use --gpg KEY"
            .to_string()
            .into());
    }
    let keys = crypto::secret_keys()?;
    let key = select("GPG key", keys, "no GPG secret key that can encrypt")?;
    Ok(key.fingerprint)
}

fn select<T: Display>(prompt: &str, items: Vec<T>, empty: &str) -> Result<T> {
    if items.is_empty() {
        return Err(empty.to_string().into());
    }
    inquire::Select::new(prompt, items)
        .with_scorer(&|input, _, option, _| score(input, option))
        .prompt()
        .map_err(|e| match e {
            InquireError::OperationCanceled | InquireError::OperationInterrupted => {
                Error::from("cancelled".to_string())
            }
            e => e.to_string().into(),
        })
}

/// fzf-style filter: the typed letters in order, with any text between them.
/// `None` hides the option; a higher score puts it nearer the top.
fn score(input: &str, option: &str) -> Option<i64> {
    let mut matcher = frizbee::Matcher::new(input, &frizbee::Config::default());
    matcher.match_one(option, 0).map(|m| m.score.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_store_is_an_error() {
        let err = select::<ProfileName>("Profile", Vec::new(), NO_PROFILES)
            .unwrap_err()
            .to_string();
        assert_eq!(err, "no profiles; create one with: duckpond create NAME");
    }

    /// The picker shows the options that `score` keeps.
    #[test]
    fn filter_matches_letters_in_order() {
        let names = ["Vault DC1", "Vault DC2", "Example DC1"];
        let shown = |input| -> Vec<&str> {
            names
                .into_iter()
                .filter(|n| score(input, n).is_some())
                .collect()
        };
        assert_eq!(shown("vadc1"), ["Vault DC1"]);
        assert_eq!(shown(""), names);
    }
}
