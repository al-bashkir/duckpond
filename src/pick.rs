//! Profile choice: completion candidates and the interactive picker.

use std::fmt::Display;
use std::io::IsTerminal;

use clap_complete::engine::{ArgValueCandidates, CompletionCandidate};
use inquire::InquireError;

use crate::crypto;
use crate::error::{Error, Result};
use crate::name::ProfileName;
use crate::store::Store;

/// Profile names for shell completion. Any error gives an empty list: no prompt, no decryption.
pub fn candidates() -> ArgValueCandidates {
    ArgValueCandidates::new(|| {
        let names = Store::from_env().and_then(|s| s.list()).unwrap_or_default();
        names
            .iter()
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
        .prompt()
        .map_err(|e| match e {
            InquireError::OperationCanceled | InquireError::OperationInterrupted => {
                Error::from("cancelled".to_string())
            }
            e => e.to_string().into(),
        })
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
}
