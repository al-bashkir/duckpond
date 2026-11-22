mod cli;
mod commands;
mod crypto;
mod error;
mod input;
mod name;
mod profile;
mod store;
mod vars;

use clap::Parser;

use cli::{Cli, Command};
use error::Result;
use store::Store;

fn run(command: Command) -> Result<()> {
    let store = Store::from_env()?;
    match command {
        Command::List { profile, reveal } => commands::list(&store, profile, reveal),
        Command::Create { profile, gpg } => commands::create(&store, profile, gpg),
        Command::Set {
            profile,
            names,
            comment,
            expires,
            keep_value,
        } => commands::set(&store, profile, names, comment, expires, keep_value),
        Command::Unset { profile, names } => commands::unset(&store, profile, names),
        Command::Delete { profile, yes } => commands::delete(&store, profile, yes),
        Command::Import { profile, file } => commands::import(&store, profile, &file),
        Command::Export {
            profile,
            names,
            output,
            force,
        } => commands::export(&store, profile, names, output, force),
        Command::Load { profile, shell } => commands::load(&store, profile, shell.into()),
        Command::Unload { profile, shell } => commands::unload(&store, profile, shell.into()),
        Command::Launch { profile, command } => commands::launch(&store, profile, command),
    }
}

fn main() {
    if let Err(e) = run(Cli::parse().command) {
        eprintln!("duckpond: error: {e}");
        std::process::exit(1);
    }
}
