//! One function per command. stdout carries data only; warnings go to stderr.

use std::io::{IsTerminal, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;

use chrono::{Local, NaiveDate};

use crate::crypto::Lock;
use crate::error::{Error, Result};
use crate::input;
use crate::name::{ProfileName, VarName};
use crate::pick;
use crate::profile::Profile;
use crate::store::{self, Store};
use crate::vars::{self, Expiry, Shell, Vars};

fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn open(store: &Store, name: ProfileName) -> Result<Profile> {
    let asked = name.clone();
    Profile::open(store, name, move || input::passphrase(&asked))
}

fn warn_expired(vars: &Vars) {
    for (name, date) in vars::expired(vars, today()) {
        eprintln!("duckpond: warning: {name} expired on {date}");
    }
}

fn print(text: &str) -> Result<()> {
    Ok(std::io::stdout().lock().write_all(text.as_bytes())?)
}

pub fn list(store: &Store, profile: Option<ProfileName>, reveal: bool) -> Result<()> {
    let Some(name) = profile else {
        let names: String = store.list()?.iter().map(|n| format!("{n}\n")).collect();
        return print(&names);
    };
    let p = open(store, name)?;
    warn_expired(&p.vars);
    print(&vars::render_table(&p.vars, reveal, today()))
}

pub fn create(store: &Store, name: ProfileName, gpg: Vec<String>) -> Result<()> {
    if store.exists(&name) {
        return Err(format!("profile \"{name}\" already exists").into());
    }
    let lock = if gpg.is_empty() {
        Lock::Passphrase(input::new_passphrase(&name)?)
    } else {
        Lock::Gpg { recipients: gpg }
    };
    Profile {
        name,
        lock,
        vars: Vars::default(),
    }
    .create(store)
}

pub fn set(
    store: &Store,
    name: ProfileName,
    names: Vec<VarName>,
    comment: Option<String>,
    expires: Option<Expiry>,
    keep_value: bool,
) -> Result<()> {
    let mut p = open(store, name)?;
    let values = match keep_value {
        true => vec![None; names.len()],
        false => input::values(&names)?.into_iter().map(Some).collect(),
    };
    for (n, v) in names.into_iter().zip(values) {
        p.vars.set(n, v, comment.clone(), expires)?;
    }
    p.save(store)
}

pub fn unset(store: &Store, name: ProfileName, names: Vec<VarName>) -> Result<()> {
    let mut p = open(store, name)?;
    p.vars.unset(&names)?;
    p.save(store)
}

pub fn delete(store: &Store, name: ProfileName, yes: bool) -> Result<()> {
    if !store.exists(&name) {
        return Err(format!("profile \"{name}\" does not exist").into());
    }
    let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    if !yes && !interactive {
        return Err(format!("not deleting \"{name}\" without a terminal; use --yes").into());
    }
    if !yes && !input::confirm(&format!("Delete profile \"{name}\"?"))? {
        return Err("aborted".to_string().into());
    }
    store.delete(&name)
}

fn read_dotenv(file: &str) -> Result<String> {
    let mut text = String::new();
    match file {
        "-" => std::io::stdin().read_to_string(&mut text).map(drop),
        path => std::fs::File::open(path).and_then(|mut f| f.read_to_string(&mut text).map(drop)),
    }
    .map_err(|e| Error::from(format!("cannot read {file}: {e}")))?;
    Ok(text)
}

pub fn import(store: &Store, name: ProfileName, file: &str) -> Result<()> {
    let pairs = vars::parse_dotenv(&read_dotenv(file)?)?;
    let mut p = open(store, name)?;
    p.vars.merge(pairs)?;
    p.save(store)
}

pub fn export(
    store: &Store,
    name: ProfileName,
    names: Vec<VarName>,
    output: Option<PathBuf>,
    force: bool,
) -> Result<()> {
    let text = vars::to_dotenv(&open(store, name)?.vars, &names)?;
    match output {
        Some(path) => store::write_atomic(&path, text.as_bytes(), force),
        None => print(&text),
    }
}

/// Shell code is only meant for `eval`; on a terminal it would be a mistake.
fn refuse_terminal(command: &str, profile: &Option<ProfileName>) -> Result<()> {
    if std::io::stdout().is_terminal() {
        let arg = profile
            .as_ref()
            .map_or("PROFILE".to_string(), |n| format!("'{n}'"));
        return Err(format!(
            "refusing to print shell code to a terminal; use: eval \"$(duckpond {command} {arg})\"; \
             or add eval \"$(duckpond init zsh)\" to your shell rc so that `duckpond {command}` works"
        )
        .into());
    }
    Ok(())
}

pub fn load(store: &Store, profile: Option<ProfileName>, shell: Shell) -> Result<()> {
    refuse_terminal("load", &profile)?;
    let p = open(store, pick::profile(store, profile)?)?;
    warn_expired(&p.vars);
    print(&vars::exports(&p.vars, shell))
}

pub fn unload(store: &Store, profile: Option<ProfileName>, shell: Shell) -> Result<()> {
    refuse_terminal("unload", &profile)?;
    let name = pick::profile(store, profile)?;
    print(&vars::unsets(&open(store, name)?.vars, shell))
}

pub fn launch(store: &Store, profile: Option<ProfileName>, command: Vec<String>) -> Result<()> {
    let name = pick::profile(store, profile)?;
    let p = open(store, name)?;
    warn_expired(&p.vars);
    let (program, args) = command
        .split_first()
        .ok_or_else(|| Error::from("missing command".to_string()))?;
    let mut cmd = std::process::Command::new(program);
    // The child must not inherit the passphrase; a profile var of that name still wins.
    cmd.args(args).env_remove("DUCKPOND_PASSPHRASE");
    cmd.envs(p.vars.0.iter().map(|(n, v)| (n.as_ref() as &str, &v.value)));
    let err = cmd.exec();
    Err(format!("cannot run {program}: {err}").into())
}
