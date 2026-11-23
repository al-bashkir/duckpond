# Changelog

All notable changes to duckpond are in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.2] - 2026-11-23

### Added

- Release binary for Linux arm64 (static, musl).

### Changed

- zsh: TAB completes a profile name with spaces in double quotes, `"Addons Mozilla"`,
  not `Addons\ Mozilla`. A word you began with a backslash keeps backslashes, and so
  does a file name with characters that are not literal in double quotes.

### Fixed

- Tab completion of a profile name with spaces now works in a quoted or escaped word, in
  bash, zsh and fish: `duckpond load "Fresh D` or `duckpond load Fresh\ D`. Before, TAB
  found nothing after it had written the common part of two names.

## [0.1.1] - 2026-11-23

### Changed

- The pickers (profile and GPG key) filter like fzf: the letters you type must come in
  that order, with any text between them, and the best match is at the top. `vadc1`
  finds `Vault DC1`.

## [0.1.0] - 2026-11-22

First release.

### Added

- Encrypted profiles of environment variables, one file per profile. A profile is locked
  with GPG (to one or more keys) or with an age passphrase.
- Commands: `list`, `create`, `set`, `unset`, `delete`, `import`, `export`, `load`,
  `unload`, `launch` and `init`.
- `create --gpg` without a key opens a menu of your GPG secret keys that can encrypt.
- A comment and an expiry date for each variable.
- `load` and `unload` for bash, zsh and fish. `launch` runs a command with a profile.
- `init` for bash, zsh and fish: `load` and `unload` that change the current shell,
  a profile picker, and Tab completion of commands, flags and profile names.
- Dotenv import and export (Docker `--env-file` rules).
- Release binaries for Linux x86_64 (static, musl) and macOS arm64, with `SHA256SUMS`.

### Security

- Values never come from command-line arguments. Prompts use `/dev/tty`, and stdout
  carries only data.
- `list` masks values unless you give `--reveal`. `list` and error messages show control
  characters escaped. `export` refuses to print a value with control characters to a
  terminal.
- `load` refuses a non-number in a variable that bash or zsh keeps as an integer.
- `launch` removes `DUCKPOND_PASSPHRASE` from the child's environment.
- Store directory mode 0700, profile files mode 0600, atomic writes.

[Unreleased]: https://github.com/al-bashkir/duckpond/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/al-bashkir/duckpond/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/al-bashkir/duckpond/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/al-bashkir/duckpond/releases/tag/v0.1.0
