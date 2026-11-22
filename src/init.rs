//! `duckpond init SHELL`: shell code for `eval`. One raw string per shell, no templating.

use std::io::Write;

use clap::ValueEnum;
use clap_complete::env::{Bash, EnvCompleter, Fish, Zsh};

use crate::error::Result;

#[derive(Clone, Copy, ValueEnum)]
pub enum InitShell {
    Bash,
    Zsh,
    Fish,
}

/// `load`/`unload` output is evaluated; with -h/--help the usage text must reach the terminal instead.
const BASH_ZSH_WRAPPER: &str = r#"duckpond() {
  case "$1" in
    load|unload)
      local __a __dp
      for __a in "$@"; do
        case "$__a" in -h|--help) command duckpond "$@"; return ;; esac
      done
      __dp="$(command duckpond "$@")" || return
      eval "$__dp" ;;
    *) command duckpond "$@" ;;
  esac
}
"#;

/// Needs compinit to have run: without `compdef` the completion code is skipped.
const ZSH_COMPDEF_OPEN: &str = "if (( $+functions[compdef] )); then\n";
const ZSH_COMPDEF_CLOSE: &str = "fi\n";

const FISH: &str = r#"function duckpond
    switch "$argv[1]"
        case load unload
            if contains -- -h $argv; or contains -- --help $argv
                command duckpond $argv
            else
                command duckpond $argv --shell fish | source
            end
        case '*'
            command duckpond $argv
    end
end
"#;

/// The command name, also the program the completion scripts call. Not argv[0]: bash puts the
/// completer inside double quotes, so a path from argv[0] would be expanded at every TAB.
pub const NAME: &str = "duckpond";

fn registration(shell: &dyn EnvCompleter) -> Result<String> {
    let mut buf = Vec::new();
    shell.write_registration("COMPLETE", NAME, NAME, NAME, &mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

pub fn script(shell: InitShell) -> Result<String> {
    Ok(match shell {
        // clap_complete's bash registration inserts candidates as is; -o filenames makes bash quote them.
        InitShell::Bash => format!(
            "{BASH_ZSH_WRAPPER}{}",
            registration(&Bash)?.replace("complete -o ", "complete -o filenames -o ")
        ),
        InitShell::Zsh => format!(
            "{BASH_ZSH_WRAPPER}{ZSH_COMPDEF_OPEN}{}{ZSH_COMPDEF_CLOSE}",
            registration(&Zsh)?
        ),
        InitShell::Fish => format!("{FISH}{}", registration(&Fish)?),
    })
}

pub fn run(shell: InitShell) -> Result<()> {
    Ok(std::io::stdout()
        .lock()
        .write_all(script(shell)?.as_bytes())?)
}
