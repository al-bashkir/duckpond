//! Integration tests: run the built binary against a temp store.
//! Every test sets DUCKPOND_DIR, HOME and GNUPGHOME to temp dirs; the real store and keyring are never used.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const PASS: &str = "correct horse battery";
const SECRET: &str = "hunter2-value-xyz";
/// Quotes, backslashes, command substitution, backticks and a newline: must reach a shell
/// variable unchanged. `\'` ends a single-quoted word in fish.
const HOSTILE: &str =
    "it's \"q\" $(touch PWNED) `touch PWNED` x\\';touch PWNED;# a\\\\b line1\nline2";
const SPACED: &str = "Fresh DB1.TEST.LOCAL";

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

struct Env {
    root: tempfile::TempDir,
    gnupg: PathBuf,
    pass: Option<String>,
}

fn run(mut c: Command, stdin: &str) -> Out {
    c.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = c.spawn().unwrap();
    // The child may exit before it reads stdin; a broken pipe is fine.
    let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
    let o = child.wait_with_output().unwrap();
    Out {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8(o.stdout).unwrap(),
        stderr: String::from_utf8(o.stderr).unwrap(),
    }
}

impl Env {
    fn new() -> Env {
        let root = tempfile::tempdir().unwrap();
        for d in ["home", "cwd", "gnupg"] {
            fs::create_dir(root.path().join(d)).unwrap();
        }
        let gnupg = root.path().join("gnupg");
        fs::set_permissions(&gnupg, fs::Permissions::from_mode(0o700)).unwrap();
        Env {
            root,
            gnupg,
            pass: Some(PASS.into()),
        }
    }

    fn store(&self) -> PathBuf {
        self.root.path().join("store")
    }

    fn cwd(&self) -> PathBuf {
        self.root.path().join("cwd")
    }

    fn command(&self, program: &str) -> Command {
        let mut c = Command::new(program);
        c.env_clear()
            .env("PATH", std::env::var("PATH").unwrap())
            .env("HOME", self.root.path().join("home"))
            .env("DUCKPOND_DIR", self.store())
            .env("GNUPGHOME", &self.gnupg)
            .current_dir(self.cwd());
        if let Some(p) = &self.pass {
            c.env("DUCKPOND_PASSPHRASE", p);
        }
        // Keep coverage data of the child binary when run under cargo llvm-cov.
        if let Some(f) = std::env::var_os("LLVM_PROFILE_FILE") {
            c.env("LLVM_PROFILE_FILE", f);
        }
        c
    }

    fn run(&self, args: &[&str], stdin: &str) -> Out {
        let mut c = self.command(env!("CARGO_BIN_EXE_duckpond"));
        c.args(args);
        run(c, stdin)
    }

    fn ok(&self, args: &[&str], stdin: &str) -> Out {
        let o = self.run(args, stdin);
        assert_eq!(o.code, 0, "{args:?} failed: {}", o.stderr);
        o
    }

    /// Runtime error: exit 1, empty stdout, `duckpond: error:` on stderr, no secret text. Returns stderr.
    fn fail(&self, args: &[&str], stdin: &str, secrets: &[&str]) -> String {
        let o = self.run(args, stdin);
        assert_eq!(o.code, 1, "{args:?}: {}", o.stderr);
        assert_eq!(o.stdout, "", "{args:?}");
        assert!(o.stderr.starts_with("duckpond: error:"), "{}", o.stderr);
        for s in secrets {
            assert!(!o.stderr.contains(s), "{args:?} leaked {s}: {}", o.stderr);
        }
        o.stderr
    }

    fn profile_file(&self, name: &str) -> PathBuf {
        self.store().join(format!("{name}.enc"))
    }
}

fn mode(p: &std::path::Path) -> u32 {
    fs::metadata(p).unwrap().permissions().mode() & 0o777
}

fn contains(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

#[test]
fn lifecycle_with_spaces_in_name() {
    let e = Env::new();
    let p = "Fresh DB1.TEST.LOCAL";
    assert_eq!(e.ok(&["list"], "").stdout, "");
    e.ok(&["create", p], "");
    assert_eq!(e.ok(&["list"], "").stdout, format!("{p}\n"));
    e.ok(&["set", p, "TOKEN"], &format!("{SECRET}\n"));
    let masked = e.ok(&["list", p], "").stdout;
    assert_eq!(masked, "NAME   VALUE\nTOKEN  ********\n");
    let shown = e.ok(&["list", p, "--reveal"], "").stdout;
    assert_eq!(shown, format!("NAME   VALUE\nTOKEN  {SECRET}\n"));
    e.ok(&["unset", p, "TOKEN"], "");
    assert_eq!(e.ok(&["list", p], "").stdout, "NAME  VALUE\n");
    e.ok(&["delete", p, "--yes"], "");
    assert_eq!(e.ok(&["list"], "").stdout, "");
}

#[test]
fn files_are_private_and_hold_no_plaintext() {
    let e = Env::new();
    e.ok(&["create", "P"], "");
    e.ok(&["set", "P", "TOKEN"], SECRET);
    assert_eq!(mode(&e.store()), 0o700);
    assert_eq!(mode(&e.profile_file("P")), 0o600);
    let bytes = fs::read(e.profile_file("P")).unwrap();
    assert!(bytes.starts_with(b"age-encryption.org/v1"));
    assert!(!contains(&bytes, SECRET));
    assert!(!contains(&bytes, PASS));
}

#[test]
fn errors_are_clean() {
    let mut e = Env::new();
    let leak = [SECRET, PASS, "wrong passphrase!"];
    e.ok(&["create", "P"], "");
    e.ok(&["set", "P", "TOKEN"], SECRET);
    let before = fs::read(e.profile_file("P")).unwrap();

    let err = e.fail(&["create", "P"], "", &leak);
    assert!(err.contains("already exists"), "{err}");
    let err = e.fail(&["create", "G", "--gpg"], "", &leak);
    assert!(err.contains("--gpg KEY"), "{err}");
    assert!(!e.profile_file("G").exists());
    e.fail(&["list", "nope"], "", &leak);
    e.fail(&["load", "nope"], "", &leak);
    e.fail(&["set", "nope", "A"], SECRET, &leak);

    // Names are checked by clap, before any file is touched.
    for bad in ["../x", "a/b", ".hidden", "-x"] {
        for cmd in ["create", "delete", "list"] {
            let o = e.run(&[cmd, bad], "");
            assert_ne!(o.code, 0, "{cmd} {bad}");
            assert_eq!(o.stdout, "");
        }
    }
    assert!(!e.root.path().join("x.enc").exists());
    assert!(!e.store().join("a").exists());
    assert_eq!(fs::read_dir(e.store()).unwrap().count(), 1);

    let o = e.run(&["set", "P", "1X"], SECRET);
    assert_ne!(o.code, 0);
    assert!(o.stderr.contains("variable name"), "{}", o.stderr);

    let err = e.fail(&["unset", "P", "NOPE"], "", &leak);
    assert!(err.contains("NOPE"), "{err}");
    assert_eq!(fs::read(e.profile_file("P")).unwrap(), before);

    e.fail(&["set", "P", "A", "B"], SECRET, &leak);
    e.fail(&["set", "P", "NOPE", "--keep-value"], "", &leak);
    assert_eq!(fs::read(e.profile_file("P")).unwrap(), before);

    for args in [&["load"][..], &["unload"], &["launch", "--", "true"]] {
        let err = e.fail(args, "", &leak);
        assert!(err.contains("missing PROFILE"), "{err}");
    }

    let err = e.fail(&["delete", "P"], "", &leak);
    assert!(err.contains("--yes"), "{err}");
    assert!(e.profile_file("P").exists());

    e.pass = Some("wrong passphrase!".into());
    for args in [&["list", "P"][..], &["load", "P"], &["export", "P"]] {
        e.fail(args, "", &leak);
    }
    assert_eq!(fs::read(e.profile_file("P")).unwrap(), before);
}

#[test]
fn comment_expiry_keep_value_and_expired_warning() {
    let e = Env::new();
    e.ok(&["create", "P"], "");
    let a = [
        "set",
        "P",
        "A",
        "--comment",
        "hello",
        "--expires",
        "2000-01-01",
    ];
    e.ok(&a, SECRET);

    let o = e.ok(&["list", "P"], "");
    assert!(o.stdout.contains("2000-01-01 (expired)"), "{}", o.stdout);
    assert!(o.stdout.contains("hello"), "{}", o.stdout);
    assert!(o.stderr.starts_with("duckpond: warning:"), "{}", o.stderr);
    assert!(o.stderr.contains("A") && o.stderr.contains("2000-01-01"));
    assert!(!o.stderr.contains(SECRET));

    let o = e.ok(&["load", "P"], "");
    assert_eq!(o.stdout, format!("export A='{SECRET}'\n"));
    assert!(o.stderr.starts_with("duckpond: warning:"), "{}", o.stderr);
    assert!(!o.stderr.contains(SECRET));

    // --keep-value reads no value (stdin is empty) and keeps the old one.
    e.ok(&["set", "P", "A", "--keep-value", "--comment", "new"], "");
    let o = e.ok(&["list", "P", "--reveal"], "");
    assert!(o.stdout.contains(SECRET) && o.stdout.contains("new"));
    assert!(o.stdout.contains("2000-01-01 (expired)"), "kept expiry");

    e.ok(&["set", "P", "A", "--keep-value", "--expires", "none"], "");
    let o = e.ok(&["load", "P"], "");
    assert_eq!(o.stderr, "");
    let o = e.ok(&["list", "P"], "");
    assert!(!o.stdout.contains("EXPIRES") && o.stdout.contains("new"));
}

#[test]
fn import_export_round_trip() {
    let e = Env::new();
    e.ok(&["create", "P"], "");
    let text = "# comment\n\nA=plain\nexport B=it's \"q\"=x\n  C=\n";
    let file = e.cwd().join("in.env");
    fs::write(&file, text).unwrap();
    e.ok(&["import", "P", file.to_str().unwrap()], "");
    e.ok(&["import", "P", "-"], "D=from stdin\nA=over\n");

    let all = "A=over\nB=it's \"q\"=x\nC=\nD=from stdin\n";
    assert_eq!(e.ok(&["export", "P"], "").stdout, all);
    assert_eq!(
        e.ok(&["export", "P", "D", "B"], "").stdout,
        "B=it's \"q\"=x\nD=from stdin\n"
    );

    let out = e.cwd().join("out.env");
    let o = out.to_str().unwrap();
    let o1 = e.ok(&["export", "P", "-o", o], "");
    assert_eq!(o1.stdout, "");
    assert_eq!(fs::read_to_string(&out).unwrap(), all);
    assert_eq!(mode(&out), 0o600);
    let err = e.fail(&["export", "P", "-o", o], "", &[]);
    assert!(err.contains("--force"), "{err}");
    e.ok(&["export", "P", "A", "-o", o, "--force"], "");
    assert_eq!(fs::read_to_string(&out).unwrap(), "A=over\n");

    // A bad line fails with its number and changes nothing.
    let before = fs::read(e.profile_file("P")).unwrap();
    let err = e.fail(&["import", "P", "-"], "OK=1\nnot a pair\n", &[]);
    assert!(err.contains("line 2"), "{err}");
    assert_eq!(fs::read(e.profile_file("P")).unwrap(), before);
    e.fail(&["import", "P", "/nonexistent/x.env"], "", &[]);

    // A value with a newline cannot be exported.
    e.ok(&["set", "P", "NL"], "a\nb");
    let err = e.fail(&["export", "P"], "", &["a\nb"]);
    assert!(err.contains("NL"), "{err}");
}

#[test]
fn load_and_unload_text() {
    let e = Env::new();
    e.ok(&["create", "P"], "");
    e.ok(&["import", "P", "-"], "A=plain\nB=it's\nC=back\\slash\n");
    let sh = "export A='plain'\nexport B='it'\\''s'\nexport C='back'\\\\'slash'\n";
    let fish = "set -gx A 'plain'\nset -gx B 'it\\'s'\nset -gx C 'back\\\\slash'\n";
    assert_eq!(e.ok(&["load", "P"], "").stdout, sh);
    assert_eq!(e.ok(&["load", "P", "--shell", "sh"], "").stdout, sh);
    assert_eq!(e.ok(&["load", "P", "--shell", "fish"], "").stdout, fish);
    assert_eq!(
        e.ok(&["unload", "P"], "").stdout,
        "unset A\nunset B\nunset C\n"
    );
    let o = e.ok(&["unload", "P", "--shell", "fish"], "");
    assert_eq!(o.stdout, "set -e A\nset -e B\nset -e C\n");
    assert_ne!(e.run(&["load", "P", "--shell", "csh"], "").code, 0);
}

fn have(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A tool a test needs. Missing: skip locally, but fail in CI (`CI` is set), so a runner
/// without gpg, zsh or bash cannot pass by skipping. fish is optional everywhere.
fn usable(program: &str) -> bool {
    if have(program) {
        return true;
    }
    let ci = std::env::var_os("CI").is_some();
    assert!(
        !ci || program == "fish",
        "{program} must be installed in CI"
    );
    eprintln!("skip {program}: not installed");
    false
}

#[test]
fn shell_eval_is_injection_safe() {
    let e = Env::new();
    let value = HOSTILE;
    e.ok(&["create", "P"], "");
    e.ok(&["set", "P", "V"], value);
    let script = r#"eval "$("$DP" load P)"; printf %s "$V"; printf '\0'; eval "$("$DP" unload P)"; printf %s "${V-unset}""#;
    let mut ran = 0;
    for shell in ["zsh", "bash"] {
        if !usable(shell) {
            continue;
        }
        ran += 1;
        let mut c = e.command(shell);
        c.env("DP", env!("CARGO_BIN_EXE_duckpond"))
            .args(["-c", script]);
        let o = run(c, "");
        assert_eq!(o.code, 0, "{shell}: {}", o.stderr);
        assert_eq!(o.stdout, format!("{value}\0unset"), "{shell}");
        assert!(!e.cwd().join("PWNED").exists(), "{shell} ran injected code");
    }
    if ran == 0 {
        eprintln!("no shell available, test skipped");
    }
}

/// Without `init`, a fish user may run the default (sh) output. It must mean the same in fish.
#[test]
fn default_load_output_is_safe_in_fish() {
    if !usable("fish") {
        return;
    }
    let e = Env::new();
    e.ok(&["create", "P"], "");
    e.ok(&["set", "P", "V"], HOSTILE);
    for script in [
        r#"eval "$("$DP" load P)"; printf %s "$V""#,
        r#""$DP" load P | source; printf %s "$V""#,
    ] {
        let mut c = e.command("fish");
        c.env("DP", env!("CARGO_BIN_EXE_duckpond"))
            .args(["-c", script]);
        let o = run(c, "");
        assert!(
            !e.cwd().join("PWNED").exists(),
            "{script}: ran injected code"
        );
        assert_eq!(o.stdout, HOSTILE, "{script}: {}", o.stderr);
    }
}

/// bash and zsh evaluate a value given to an integer parameter as arithmetic, and a
/// subscript in it runs command substitution: quoting cannot help, so `load` must refuse.
#[test]
fn load_refuses_arithmetic_in_shell_integer_names() {
    let e = Env::new();
    e.ok(&["create", "P"], "");
    // OPTIND is an integer in bash, HISTSIZE in zsh.
    let hostile = "OPTIND=zz[$(touch PWNED)0]\nHISTSIZE=PPID[$(touch PWNED)1]\n";
    e.ok(&["import", "P", "-"], hostile);
    for shell in ["zsh", "bash"] {
        if !usable(shell) {
            continue;
        }
        let mut c = e.command(shell);
        c.env("DP", env!("CARGO_BIN_EXE_duckpond"))
            .args(["-c", r#"eval "$("$DP" load P)""#]);
        run(c, "");
        assert!(!e.cwd().join("PWNED").exists(), "{shell} ran injected code");
    }
    let err = e.fail(&["load", "P"], "", &["PWNED"]);
    assert!(err.contains("HISTSIZE"), "{err}");
    // fish has no integer parameters.
    e.ok(&["load", "P", "--shell", "fish"], "");

    e.ok(&["import", "P", "-"], "OPTIND=1\nHISTSIZE=-5000\n");
    assert_eq!(
        e.ok(&["load", "P"], "").stdout,
        "export HISTSIZE='-5000'\nexport OPTIND='1'\n"
    );
}

/// Errors carry names from the command line (a glob can pass a crafted file name): the
/// terminal must not act on control characters in them, and a line feed must not start a fake line.
#[test]
fn errors_escape_control_characters() {
    let e = Env::new();
    e.ok(&["create", "P"], "");
    let name = "x\u{1b}]0;T\u{7}\r\u{9b}2J\nduckpond: done";
    fs::write(e.cwd().join(name), "").unwrap();
    let missing = format!("missing{name}");
    for args in [
        &["import", "P", &missing][..],
        &["launch", "P", "--", name],
        &["export", "P", "-o", name],
    ] {
        let err = e.fail(args, "", &["\u{1b}", "\u{7}", "\r", "\u{9b}"]);
        assert_eq!(err.lines().count(), 1, "{err}");
        assert!(err.contains("x\\u{1b}]0;T"), "{err}");
    }
    // A two-line message from a dependency (age) stays readable on one line.
    fs::write(e.profile_file("P"), "age-encryption.org/v1\nzz").unwrap();
    let err = e.fail(&["list", "P"], "", &["\\n"]);
    assert!(err.contains("format.; Have"), "{err}");
}

#[test]
fn launch_runs_with_vars_and_propagates_exit() {
    let e = Env::new();
    let value = "it's \"q\" $HOME\nline2";
    e.ok(&["create", "P"], "");
    e.ok(&["set", "P", "V"], value);

    let show = r#"printf %s "$V"; printf '\0'; printf %s "${DUCKPOND_PASSPHRASE-unset}""#;
    let o = e.ok(&["launch", "P", "--", "sh", "-c", show], "");
    assert_eq!(o.stdout, format!("{value}\0unset"));

    let o = e.run(&["launch", "P", "--", "sh", "-c", "exit 7"], "");
    assert_eq!(o.code, 7);

    let err = e.fail(&["launch", "P", "--", "/nonexistent/cmd"], "", &[value]);
    assert!(err.contains("cannot run /nonexistent/cmd:"), "{err}");
}

#[test]
fn gpg_lock_round_trip() {
    if !usable("gpg") {
        return;
    }
    // Short path: gpg-agent sockets live in GNUPGHOME and have a length limit.
    let home = tempfile::Builder::new()
        .prefix("dp-gpg")
        .tempdir_in("/tmp")
        .unwrap();
    fs::set_permissions(home.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let mut e = Env::new();
    e.gnupg = home.path().to_path_buf();
    e.pass = None;

    let gen_args = [
        "--batch",
        "--passphrase",
        "",
        "--quick-gen-key",
        "duckpond test <t@example.invalid>",
        "default",
        "default",
        "never",
    ];
    let mut keygen = e.command("gpg");
    keygen.args(gen_args);
    let g = run(keygen, "");
    assert_eq!(g.code, 0, "gpg keygen: {}", g.stderr);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        e.ok(&["create", "P", "--gpg", "t@example.invalid"], "");
        e.ok(&["set", "P", "V"], SECRET);
        let o = e.ok(&["list", "P", "--reveal"], "");
        assert!(o.stdout.contains(SECRET), "{}", o.stdout);
        // The second write re-encrypts to the recipients found while decrypting.
        e.ok(&["set", "P", "W"], "second");
        let o = e.ok(&["load", "P"], "");
        assert_eq!(
            o.stdout,
            format!("export V='{SECRET}'\nexport W='second'\n")
        );
        let bytes = fs::read(e.profile_file("P")).unwrap();
        assert!(!bytes.starts_with(b"age-encryption.org"));
    }));

    let mut kill = e.command("gpgconf");
    kill.args(["--kill", "gpg-agent"]);
    run(kill, "");
    if let Err(p) = result {
        std::panic::resume_unwind(p);
    }
}

#[test]
fn version() {
    let e = Env::new();
    let want = concat!("duckpond ", env!("CARGO_PKG_VERSION"), "\n");
    assert_eq!(e.ok(&["--version"], "").stdout, want);
}

#[test]
fn help_is_grouped_with_and_without_args() {
    let e = Env::new();
    let help = e.ok(&["--help"], "").stdout;
    let bare = e.run(&[], "");
    assert_eq!(bare.code, 2);
    assert_eq!(bare.stderr, help, "no args prints the same help");
    assert!(help.contains("Profiles:\n  list"), "{help}");
    assert!(!help.contains('\x1b'), "no color codes when piped");
}

/// Runs `script` in `shell` with `duckpond` on PATH as a real command, so `init` output wraps it.
fn run_in_shell(e: &Env, shell: &str, script: &str) -> Out {
    let bin_dir = std::path::Path::new(env!("CARGO_BIN_EXE_duckpond"))
        .parent()
        .unwrap();
    let path = format!("{}:{}", bin_dir.display(), std::env::var("PATH").unwrap());
    let mut c = e.command(shell);
    c.env("PATH", path).args(["-c", script]);
    run(c, "")
}

const SH_SESSION: &str = r#"eval "$(duckpond init @SH@)"
duckpond load 'Fresh DB1.TEST.LOCAL'; printf %s "$V"; printf '\0'
duckpond unload 'Fresh DB1.TEST.LOCAL'; printf %s "${V-unset}"; printf '\0'
duckpond list; printf '\0'
duckpond load --help; printf '\0'
duckpond unload -h; printf '\0'
printf %s "${V-unset}"; printf '\0'
COMPLETE=@SH@ _CLAP_COMPLETE_INDEX=2 duckpond -- duckpond load Fre"#;

/// Same session as SH_SESSION. Runs where fish is installed (not in CI).
const FISH_SESSION: &str = r#"duckpond init fish | source
duckpond load 'Fresh DB1.TEST.LOCAL'; printf %s "$V"; printf '\0'
duckpond unload 'Fresh DB1.TEST.LOCAL'; set -q V; or printf unset; printf '\0'
duckpond list; printf '\0'
duckpond load --help; printf '\0'
duckpond unload -h; printf '\0'
set -q V; or printf unset"#;

#[test]
fn init_wrappers_eval_help_and_pass_through() {
    let e = Env::new();
    e.ok(&["create", SPACED], "");
    e.ok(&["set", SPACED, "V"], HOSTILE);
    for shell in ["zsh", "bash", "fish"] {
        if !usable(shell) {
            continue;
        }
        let stub = "compdef() { print -r -- \"$*\" >> compdefs; }\n";
        let script = match shell {
            "fish" => FISH_SESSION.to_string(),
            "zsh" => format!("{stub}{}", SH_SESSION.replace("@SH@", shell)),
            _ => SH_SESSION.replace("@SH@", shell),
        };
        let o = run_in_shell(&e, shell, &script);
        assert_eq!(o.code, 0, "{shell}: {}", o.stderr);
        assert_eq!(o.stderr, "", "{shell}");
        let parts: Vec<&str> = o.stdout.split('\0').collect();
        assert_eq!(
            parts[..3],
            [HOSTILE, "unset", "Fresh DB1.TEST.LOCAL\n"],
            "{shell}"
        );
        assert!(
            parts[3].contains("Usage: duckpond load"),
            "{shell}: {}",
            parts[3]
        );
        assert!(
            parts[4].contains("Usage: duckpond unload"),
            "{shell}: {}",
            parts[4]
        );
        assert_eq!(parts[5], "unset", "{shell}: help output was evaluated");
        if shell != "fish" {
            // The wrapper function sits between the shell and the completer binary.
            assert_eq!(parts[6], "Fresh DB1.TEST.LOCAL", "{shell}");
        }
        assert!(!e.cwd().join("PWNED").exists(), "{shell} ran injected code");
        if shell == "zsh" {
            let log = fs::read_to_string(e.cwd().join("compdefs")).unwrap();
            assert!(log.contains("duckpond"), "{log}");
        }
    }
}

/// Run the binary as the registered completion script does; returns the candidates.
fn complete(e: &Env, shell: &str, words: &[&str]) -> Vec<String> {
    let sep = if shell == "zsh" { "\n" } else { "\x0b" };
    let index = words.len(); // the shell passes the program name as word 0
    let mut c = e.command(env!("CARGO_BIN_EXE_duckpond"));
    c.env("COMPLETE", shell)
        .env("_CLAP_IFS", sep)
        .env("_CLAP_COMPLETE_INDEX", index.to_string())
        .env("_CLAP_COMPLETE_COMP_TYPE", "9")
        .env("_CLAP_COMPLETE_SPACE", "false")
        .arg("--")
        .args(["duckpond"])
        .args(words);
    let o = run(c, "");
    assert_eq!(o.code, 0, "{shell} {words:?}: {}", o.stderr);
    o.stdout.split(sep).map(String::from).collect()
}

#[test]
fn completion_lists_profile_names_without_a_passphrase() {
    let mut e = Env::new();
    e.ok(&["create", SPACED], "");
    e.ok(&["create", "Openly"], "");
    e.pass = None;
    for shell in ["zsh", "bash"] {
        for sub in [
            "load", "unload", "launch", "list", "set", "unset", "delete", "import", "export",
        ] {
            let all = complete(&e, shell, &[sub, ""]);
            for name in [SPACED, "Openly"] {
                assert!(all.iter().any(|c| c == name), "{shell} {sub}: {all:?}");
            }
        }
        assert_eq!(complete(&e, shell, &["load", "Fre"]), [SPACED], "{shell}");
        // `create` names a new profile: no profile candidates.
        let create = complete(&e, shell, &["create", ""]);
        assert!(!create.iter().any(|c| c == SPACED), "{shell}: {create:?}");
    }
}

/// The shell passes the word as typed, quoting included. After TAB writes the common part of
/// two names (`Fresh\ DB`), the next TAB must still find both; so must a word begun with a quote.
#[test]
fn completion_reads_a_quoted_word() {
    let mut e = Env::new();
    e.ok(&["create", "Fresh DB1"], "");
    e.ok(&["create", "Fresh DB2"], "");
    e.pass = None;
    for shell in ["zsh", "bash"] {
        for word in [r"Fresh\ D", "\"Fresh D", "'Fresh D", "\"Fresh DB1\""] {
            let found = complete(&e, shell, &["load", word]);
            let want: &[&str] = if word.ends_with('"') {
                &["Fresh DB1"]
            } else {
                &["Fresh DB1", "Fresh DB2"]
            };
            assert_eq!(found, want, "{shell} {word}");
        }
    }
}

/// Types `line` and TAB in an interactive zsh with `init zsh` loaded; returns the line after TAB.
fn zsh_tab(e: &Env, line: &str) -> String {
    let script = r#"rm -f buf; zmodload zsh/zpty
zpty z 'zsh -f -i'
zpty -w z 'autoload -Uz compinit; compinit -u -D; eval "$(duckpond init zsh)"
dump() { print -rn -- "$BUFFER" > buf; exit }; zle -N dump; bindkey "^X" dump'
zpty -w -n z '@LINE@'$'\t\x18'
for i in {1..100}; do [[ -e buf ]] && break; sleep 0.1; done
zpty -d z; cat buf"#;
    let o = run_in_shell(e, "zsh", &script.replace("@LINE@", line));
    assert_eq!(o.code, 0, "{line}: {}", o.stderr);
    o.stdout
}

/// zsh writes a name with a space in double quotes, as a user would type it. A word the user
/// began with a backslash, and a value with characters that are not literal in double
/// quotes, keep zsh's backslashes.
#[test]
fn zsh_completion_puts_spaced_names_in_double_quotes() {
    if !usable("zsh") {
        return;
    }
    let mut e = Env::new();
    e.ok(&["create", "Addons Mozilla"], "");
    e.ok(&["create", "Openly"], "");
    e.pass = None;
    fs::write(e.cwd().join("b c$(touch PWNED)"), "").unwrap();
    for (typed, want) in [
        ("duckpond list Ad", r#"duckpond list "Addons Mozilla" "#),
        ("duckpond list Op", "duckpond list Openly "),
        (
            r"duckpond list Addons\ M",
            r"duckpond list Addons\ Mozilla ",
        ),
        (
            "duckpond export Openly --output b",
            r"duckpond export Openly --output b\ c\$\(touch\ PWNED\) ",
        ),
    ] {
        assert_eq!(zsh_tab(&e, typed), want, "{typed}");
    }
    assert!(!e.cwd().join("PWNED").exists());
}

/// Without `-o filenames`, bash inserts a file candidate unquoted: a name such as
/// `out.env;touch X` in a shared directory becomes code on the command line.
#[test]
fn bash_completion_quotes_file_names() {
    let e = Env::new();
    let script = e.ok(&["init", "bash"], "").stdout;
    let completes: Vec<&str> = script
        .lines()
        .filter(|l| l.trim_start().starts_with("complete "))
        .collect();
    assert!(!completes.is_empty(), "{script}");
    for line in completes {
        assert!(line.contains(" -o filenames "), "{line}");
    }
    if usable("bash") {
        let o = run_in_shell(
            &e,
            "bash",
            r#"eval "$(duckpond init bash)"; complete -p duckpond"#,
        );
        assert_eq!(o.code, 0, "{}", o.stderr);
        assert!(o.stdout.contains("-o filenames"), "{}", o.stdout);
    }
}

/// The bash script calls its completer inside double quotes: a path taken from argv[0], such
/// as a directory named `x$(touch PWNED)y`, would run at every TAB.
#[test]
fn completion_script_calls_duckpond_by_name() {
    use std::os::unix::process::CommandExt;
    let e = Env::new();
    if !usable("bash") {
        return;
    }
    let print = |args: &[&str], complete: &str| {
        let mut c = e.command(env!("CARGO_BIN_EXE_duckpond"));
        c.arg0("x$(touch PWNED)y/duckpond").args(args);
        if !complete.is_empty() {
            c.env("COMPLETE", complete);
        }
        let o = run(c, "");
        assert_eq!(o.code, 0, "{args:?}: {}", o.stderr);
        o.stdout
    };
    let call = ". ./reg.bash; COMP_WORDS=(duckpond lis); COMP_CWORD=1; COMP_TYPE=9
        _clap_complete_duckpond duckpond lis duckpond; printf %s \"${COMPREPLY[*]}\"";
    for (how, script) in [
        ("init bash", print(&["init", "bash"], "")),
        ("COMPLETE=bash", print(&[], "bash")),
    ] {
        fs::write(e.cwd().join("reg.bash"), script).unwrap();
        let o = run_in_shell(&e, "bash", call);
        assert!(!e.cwd().join("PWNED").exists(), "{how}: argv[0] ran");
        assert_eq!(o.stdout, "list", "{how}: {}", o.stderr);
    }
}

#[test]
fn init_prints_one_script_per_shell() {
    let e = Env::new();
    for (shell, needles) in [
        ("zsh", &["_clap_dynamic_completer_duckpond"][..]),
        ("bash", &["_clap_complete_duckpond"]),
        ("fish", &["function duckpond", "--command duckpond"]),
    ] {
        let out = e.ok(&["init", shell], "").stdout;
        for n in needles {
            assert!(out.contains(n), "{shell}: missing {n}");
        }
    }
    assert_ne!(e.run(&["init", "csh"], "").code, 0);
}
