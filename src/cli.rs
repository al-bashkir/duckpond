use std::str::FromStr;

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};

use crate::error::Error;
use crate::init::InitShell;
use crate::name::{ProfileName, VarName};
use crate::pick;
use crate::vars::{Expiry, Shell};

#[derive(Parser)]
#[command(version, about = "Encrypted environment-variable profiles")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// Command groups of the top-level help. Every subcommand is in exactly one group (tested).
const GROUPS: &[(&str, &[&str])] = &[
    ("Profiles", &["list", "create", "delete"]),
    ("Variables", &["set", "unset", "get", "import", "export"]),
    ("Shell", &["load", "unload", "launch"]),
    ("Setup", &["init"]),
];

/// The CLI definition, with the top-level help listing commands by group.
/// clap has one heading for all subcommands, so the groups go into the help template.
pub fn command() -> clap::Command {
    let cmd = Cli::command();
    let template = help_template(&cmd);
    cmd.help_template(template)
}

fn help_template(cmd: &clap::Command) -> String {
    let styles = cmd.get_styles();
    let (h, l) = (styles.get_header(), styles.get_literal());
    let width = GROUPS
        .iter()
        .flat_map(|(_, n)| n.iter())
        .map(|n| n.len())
        .max();
    let width = width.unwrap_or(0);
    let mut out = String::from("{before-help}{about-with-newline}\n{usage-heading} {usage}\n\n");
    for (group, names) in GROUPS {
        out += &format!("{h}{group}:{h:#}\n");
        for name in *names {
            let about = cmd.find_subcommand(name).and_then(|c| c.get_about());
            let about = about.map(|a| a.to_string()).unwrap_or_default();
            out += &format!("  {l}{name:<width$}{l:#}  {about}\n");
        }
        out.push('\n');
    }
    out + &format!("{h}Options:{h:#}\n{{options}}{{after-help}}")
}

#[derive(Subcommand)]
pub enum Command {
    /// List profiles, or show the variables of one profile
    List {
        /// Profile to show; without it, list all profile names
        #[arg(value_parser = parse::<ProfileName>, add = pick::candidates())]
        profile: Option<ProfileName>,
        /// Show values instead of ********
        #[arg(long)]
        reveal: bool,
    },
    /// Create an empty profile
    Create {
        /// Name of the new profile
        #[arg(value_parser = parse::<ProfileName>)]
        profile: ProfileName,
        /// Lock with this GPG key (repeatable); --gpg alone picks one of your secret keys;
        /// without --gpg, lock with a passphrase
        #[arg(
            long = "gpg",
            value_name = "KEY",
            num_args = 0..=1,
            default_missing_value = "",
            value_parser = parse_gpg
        )]
        gpg: Vec<GpgArg>,
    },
    /// Add or change variables; values come from the terminal prompt or stdin
    Set {
        /// Profile to change
        #[arg(value_parser = parse::<ProfileName>, add = pick::candidates())]
        profile: ProfileName,
        /// Variables to set
        #[arg(required = true, value_parser = parse::<VarName>)]
        names: Vec<VarName>,
        /// Comment text; an empty text removes the comment
        #[arg(long, value_name = "TEXT")]
        comment: Option<String>,
        /// Expiry date YYYY-MM-DD, or "none" to remove it
        #[arg(long, value_name = "DATE", value_parser = parse::<Expiry>)]
        expires: Option<Expiry>,
        /// Change only the comment and expiry; read no value
        #[arg(long)]
        keep_value: bool,
    },
    /// Remove variables from a profile
    Unset {
        /// Profile to change
        #[arg(value_parser = parse::<ProfileName>, add = pick::candidates())]
        profile: ProfileName,
        /// Variables to remove
        #[arg(required = true, value_parser = parse::<VarName>)]
        names: Vec<VarName>,
    },
    /// Print the value of one variable as it is: no quotes, no newline
    Get {
        /// Profile to read
        #[arg(value_parser = parse::<ProfileName>, add = pick::candidates())]
        profile: Option<ProfileName>,
        /// Variable to print; without it, pick one
        #[arg(value_parser = parse::<VarName>)]
        name: Option<VarName>,
    },
    /// Delete a profile
    Delete {
        /// Profile to delete
        #[arg(value_parser = parse::<ProfileName>, add = pick::candidates())]
        profile: ProfileName,
        /// Delete without asking
        #[arg(long)]
        yes: bool,
    },
    /// Merge variables from a dotenv file into a profile
    Import {
        /// Profile to change
        #[arg(value_parser = parse::<ProfileName>, add = pick::candidates())]
        profile: ProfileName,
        /// Dotenv file to read, or - for stdin
        file: String,
    },
    /// Write the variables of a profile as dotenv
    Export {
        /// Profile to read
        #[arg(value_parser = parse::<ProfileName>, add = pick::candidates())]
        profile: ProfileName,
        /// Export only these variables
        #[arg(value_parser = parse::<VarName>)]
        names: Vec<VarName>,
        /// Write to this file (mode 0600) instead of stdout
        #[arg(short, long, value_name = "FILE")]
        output: Option<std::path::PathBuf>,
        /// Overwrite an existing output file
        #[arg(long, requires = "output")]
        force: bool,
    },
    /// Print shell code that sets the variables of a profile
    Load {
        /// Profile to load
        #[arg(value_parser = parse::<ProfileName>, add = pick::candidates())]
        profile: Option<ProfileName>,
        /// Shell syntax of the output
        #[arg(long, value_enum, default_value = "sh")]
        shell: ShellArg,
    },
    /// Print shell code that unsets the variables of a profile
    Unload {
        /// Profile to unload
        #[arg(value_parser = parse::<ProfileName>, add = pick::candidates())]
        profile: Option<ProfileName>,
        /// Shell syntax of the output
        #[arg(long, value_enum, default_value = "sh")]
        shell: ShellArg,
    },
    /// Run a command with the variables of a profile
    Launch {
        /// Profile to use
        #[arg(value_parser = parse::<ProfileName>, add = pick::candidates())]
        profile: Option<ProfileName>,
        /// Command and arguments, after --
        #[arg(last = true, required = true, value_name = "CMD")]
        command: Vec<String>,
    },
    /// Print shell code: a `duckpond` wrapper that evaluates load/unload, and completion
    Init {
        /// Shell to print code for
        #[arg(value_enum)]
        shell: InitShell,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ShellArg {
    Sh,
    Fish,
}

impl From<ShellArg> for Shell {
    fn from(s: ShellArg) -> Shell {
        match s {
            ShellArg::Sh => Shell::Sh,
            ShellArg::Fish => Shell::Fish,
        }
    }
}

/// Our `Error` is not a `std::error::Error`, so clap needs a `String` error.
fn parse<T: FromStr<Err = Error>>(s: &str) -> Result<T, String> {
    s.parse().map_err(|e: Error| e.to_string())
}

/// One `--gpg` occurrence: a key given by the user, or `--gpg` alone (pick in a menu).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpgArg {
    Pick,
    Key(String),
}

fn parse_gpg(s: &str) -> Result<GpgArg, String> {
    Ok(match s {
        "" => GpgArg::Pick,
        key => GpgArg::Key(key.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn every_command_is_in_exactly_one_group() {
        let mut defined: Vec<String> = Cli::command()
            .get_subcommands()
            .map(|c| c.get_name().to_string())
            .collect();
        let mut grouped: Vec<String> = GROUPS
            .iter()
            .flat_map(|(_, names)| names.iter().map(|n| n.to_string()))
            .collect();
        defined.sort();
        grouped.sort();
        assert_eq!(grouped, defined);
    }

    #[test]
    fn top_help_lists_commands_by_group() {
        let help = command().render_help().to_string();
        let start = help.find("Profiles:").expect(&help);
        let groups = &help[start..help.find("Options:").expect(&help)];
        let headings: Vec<&str> = groups.lines().filter(|l| l.ends_with(':')).collect();
        assert_eq!(headings, ["Profiles:", "Variables:", "Shell:", "Setup:"]);
        assert!(
            groups.contains("Profiles:\n  list    List profiles"),
            "{groups}"
        );
        assert!(
            groups.contains("Shell:\n  load    Print shell code"),
            "{groups}"
        );
        assert!(!help.contains("Commands:"), "{help}");
        assert!(help.contains("-V, --version"), "{help}");
    }

    #[test]
    fn subcommand_help_keeps_the_default_layout() {
        let mut cmd = command();
        let help = cmd
            .find_subcommand_mut("create")
            .unwrap()
            .render_help()
            .to_string();
        assert!(
            help.contains("Options:") && !help.contains("Profiles:"),
            "{help}"
        );
    }

    #[test]
    fn gpg_without_key_means_pick() {
        let gpg = |args: &[&str]| match Cli::try_parse_from(args).unwrap().command {
            Command::Create { gpg, .. } => gpg,
            _ => panic!("not create"),
        };
        let key = |k: &str| GpgArg::Key(k.into());
        assert_eq!(gpg(&["duckpond", "create", "P"]), []);
        assert_eq!(gpg(&["duckpond", "create", "P", "--gpg"]), [GpgArg::Pick]);
        assert_eq!(
            gpg(&["duckpond", "create", "P", "--gpg", "k@x"]),
            [key("k@x")]
        );
        assert_eq!(
            gpg(&["duckpond", "create", "P", "--gpg", "--gpg=k@x"]),
            [GpgArg::Pick, key("k@x")]
        );
    }

    #[test]
    fn launch_without_profile() {
        let cli = Cli::try_parse_from(["duckpond", "launch", "--", "nvim", "-c", "x"]).unwrap();
        let Command::Launch { profile, command } = cli.command else {
            panic!("not launch");
        };
        assert!(profile.is_none());
        assert_eq!(command, ["nvim", "-c", "x"]);
    }

    #[test]
    fn launch_with_profile_and_no_command_fails() {
        assert!(Cli::try_parse_from(["duckpond", "launch", "P"]).is_err());
        assert!(Cli::try_parse_from(["duckpond", "launch", "P", "--"]).is_err());
    }
}
