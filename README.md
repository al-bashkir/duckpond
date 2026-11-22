# duckpond

Keep environment variables in encrypted profiles. Load them into your shell, or run a
command with them, only when you need them.

## Install

```sh
cargo install --path .
```

Or download a binary from the GitHub releases (Linux x86_64 static, macOS arm64) and check
it against `SHA256SUMS`.

Needs `gpg` on `PATH` for GPG profiles. Linux and macOS.

With a terminal pinentry (curses or tty), gpg must know your terminal, else it fails with
`Inappropriate ioctl for device`. Put this in your shell rc (a GUI pinentry does not need it):

```sh
export GPG_TTY=$(tty)
```

## Shell setup

Add this to your shell rc file. For zsh, put it after `compinit`.

```sh
eval "$(duckpond init zsh)"      # or: bash
duckpond init fish | source      # fish
```

This gives you:

- `duckpond load` and `duckpond unload` that change the current shell. Without a profile
  name, they open a picker (type to filter).
- Tab completion of commands, flags and profile names.

Tested with zsh and bash. The fish script has not been run yet.

## Usage

```sh
duckpond create work --gpg you@example.com   # GPG lock (no prompt if gpg-agent has the key)
duckpond create work --gpg                   # pick one of your GPG secret keys in a menu
duckpond create home                         # passphrase lock (asks twice)

duckpond set work GITHUB_TOKEN               # asks for the value, hidden
pass show gh | duckpond set work GITHUB_TOKEN    # or reads it from stdin
duckpond set work GITHUB_TOKEN --comment "CI bot" --expires 2027-01-31
duckpond set work GITHUB_TOKEN --keep-value --expires none

duckpond list                    # profile names
duckpond list work               # NAME / VALUE (masked) / EXPIRES / COMMENT
duckpond list work --reveal      # show values

duckpond load work               # load into this shell
duckpond load                    # pick a profile, then load it
duckpond unload work
duckpond launch work -- terraform plan
duckpond launch -- nvim          # pick a profile, then run nvim

duckpond import work .env        # merge a dotenv file (or - for stdin)
duckpond export work -o out.env  # dotenv, mode 0600; --force to replace
duckpond unset work OLD_VAR
duckpond delete work             # asks; --yes when there is no terminal
```

`--gpg` takes an optional value, so put it after the profile name: `create --gpg work`
reads `work` as the key. The menu shows only keys that can encrypt (not expired, revoked
or disabled), and the profile is locked to the fingerprint you choose.

Without `init`, use `eval "$(duckpond load work)"`. `load` refuses to print to a terminal,
so secrets do not land in your scrollback by mistake.

## Storage

- Directory: `$DUCKPOND_DIR`, else `${XDG_DATA_HOME:-~/.local/share}/duckpond`. Mode 0700.
- One file per profile: `<name>.enc`, mode 0600. Writes are atomic (temp file, fsync, rename).
- The file is a GPG message or an age passphrase file. The lock type is read from the file.
- Inside the encryption: a JSON map of variables (value, comment, expiry). No keys,
  no passphrase, no recipients.
- Profile names are file names, so `list` and completion need no decryption.
  Variable names and values are only inside the encryption.

## Security notes

- Values never come from command-line arguments, so they do not reach shell history or `ps`.
- Prompts read and write `/dev/tty`. stdout carries only data, so `eval` never runs a prompt,
  a warning or an error message.
- `list` masks values unless you give `--reveal`. Control characters are shown escaped,
  so a value cannot send escape sequences to your terminal. `export` without `-o` refuses
  to print a value with control characters to a terminal; a pipe or `-o` gets it unchanged.
- Profile names allow only `A-Z a-z 0-9 space . _ @ + -`, start with a letter or digit,
  and contain no path separators.
- `DUCKPOND_PASSPHRASE` skips the passphrase prompt (for scripts). Other processes of
  your user can read the environment, so use it with care. `launch` removes it from the
  child's environment.
- `load` refuses a profile in which a variable that bash or zsh keeps as an integer
  (`OPTIND`, `HISTSIZE`, `SHLVL` and others) holds anything but a plain number. These shells
  evaluate such a value as arithmetic, and that can run commands. A variable that you
  declare as an integer in your own shell rc file is not on this list.
- Dotenv import/export is raw `NAME=value` per line (Docker `--env-file` rules: no quote
  removal). Export refuses values that contain line breaks.

## Release

CI (fmt, clippy, tests, `cargo audit`) runs on every push that changes `src/`, `tests/`,
`Cargo.toml`, `Cargo.lock`, `.cargo/` or a workflow. Dependabot opens weekly pull requests for
GitHub Actions and crate updates. To release, set the new version in `Cargo.toml`, commit, then push a tag with
the same version:

```sh
git tag v0.2.0 && git push origin v0.2.0
```

The release workflow runs CI again, stops if the tag and `Cargo.toml` differ, builds the
binaries and publishes them with `SHA256SUMS` as a GitHub release.

## Glossary

- **profile**: a named, encrypted set of variables. One file in the store.
- **variable**: a name with a value, and an optional comment and expiry date.
- **lock**: how a profile is encrypted. `gpg` (to one or more keys) or `passphrase` (age).
- **store**: the directory that holds the profile files.
