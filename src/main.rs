mod cli;
mod commands;
mod crypto;
mod error;
mod init;
mod input;
mod name;
mod pick;
mod profile;
mod store;
mod vars;

use clap::FromArgMatches;
use clap_complete::CompleteEnv;

use cli::{Cli, Command};
use error::Result;
use store::Store;

fn run(command: Command) -> Result<()> {
    let store = Store::from_env; // opened per arm: `init` must work without a store
    match command {
        Command::List { profile, reveal } => commands::list(&store()?, profile, reveal),
        Command::Create { profile, gpg } => commands::create(&store()?, profile, gpg),
        Command::Set {
            profile,
            names,
            comment,
            expires,
            keep_value,
        } => commands::set(&store()?, profile, names, comment, expires, keep_value),
        Command::Unset { profile, names } => commands::unset(&store()?, profile, names),
        Command::Delete { profile, yes } => commands::delete(&store()?, profile, yes),
        Command::Import { profile, file } => commands::import(&store()?, profile, &file),
        Command::Export {
            profile,
            names,
            output,
            force,
        } => commands::export(&store()?, profile, names, output, force),
        Command::Load { profile, shell } => commands::load(&store()?, profile, shell.into()),
        Command::Unload { profile, shell } => commands::unload(&store()?, profile, shell.into()),
        Command::Launch { profile, command } => commands::launch(&store()?, profile, command),
        Command::Init { shell } => init::run(shell),
    }
}

fn main() {
    CompleteEnv::with_factory(cli::command)
        .completer(init::NAME)
        .complete();
    let cli = Cli::from_arg_matches(&cli::command().get_matches()).unwrap_or_else(|e| e.exit());
    if let Err(e) = run(cli.command) {
        eprintln!("duckpond: error: {e}");
        std::process::exit(1);
    }
}
