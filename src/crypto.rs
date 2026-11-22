use std::io::{Read, Write};
use std::process::{Command, Stdio};

use age::secrecy::SecretString;

use crate::error::Result;

const AGE_MAGIC: &[u8] = b"age-encryption.org/v1\n";

#[derive(Debug, Clone)]
pub enum Lock {
    Gpg { recipients: Vec<String> },
    Passphrase(SecretString),
}

pub fn encrypt(lock: &Lock, plain: &[u8]) -> Result<Vec<u8>> {
    match lock {
        Lock::Passphrase(p) => age_encrypt(p.clone(), plain),
        Lock::Gpg { recipients } => gpg_encrypt(recipients, plain),
    }
}

pub fn decrypt(
    cipher: &[u8],
    passphrase: impl FnOnce() -> Result<SecretString>,
) -> Result<(Vec<u8>, Lock)> {
    if cipher.starts_with(AGE_MAGIC) {
        let p = age_check(cipher).and_then(|()| passphrase())?;
        let plain = age_decrypt(p.clone(), cipher)?;
        return Ok((plain, Lock::Passphrase(p)));
    }
    gpg_decrypt(cipher)
}

// Fixed scrypt cost, as in Go age. age's default calibrates by timing on each run,
// so a file made on an idle machine could fail "excessive work" on a busy or slow one.
const WORK_FACTOR: u8 = 18;
const MAX_WORK_FACTOR: u8 = 22;

fn age_encrypt(passphrase: SecretString, plain: &[u8]) -> Result<Vec<u8>> {
    let mut recipient = age::scrypt::Recipient::new(passphrase);
    recipient.set_work_factor(WORK_FACTOR);
    let encryptor = age::Encryptor::with_recipients(std::iter::once(&recipient as _))
        .map_err(|e| format!("cannot encrypt: {e}"))?;
    let mut out = Vec::new();
    let mut w = encryptor.wrap_output(&mut out)?;
    w.write_all(plain)?;
    w.finish()?;
    Ok(out)
}

fn age_check(cipher: &[u8]) -> Result<()> {
    let d = age::Decryptor::new_buffered(cipher).map_err(|e| format!("bad age file: {e}"))?;
    if d.is_scrypt() {
        return Ok(());
    }
    Err("only passphrase age files are supported".to_string().into())
}

fn age_decrypt(passphrase: SecretString, cipher: &[u8]) -> Result<Vec<u8>> {
    let d = age::Decryptor::new_buffered(cipher).map_err(|e| format!("bad age file: {e}"))?;
    let mut id = age::scrypt::Identity::new(passphrase);
    id.set_max_work_factor(MAX_WORK_FACTOR);
    let mut r = d
        .decrypt(std::iter::once(&id as &dyn age::Identity))
        .map_err(|e| format!("cannot decrypt: {e}"))?;
    let mut plain = Vec::new();
    r.read_to_end(&mut plain)
        .map_err(|e| format!("cannot decrypt: {e}"))?;
    Ok(plain)
}

fn gpg_encrypt(recipients: &[String], plain: &[u8]) -> Result<Vec<u8>> {
    if recipients.is_empty() {
        return Err("no gpg recipients".to_string().into());
    }
    let mut args = vec!["--batch", "--quiet", "--yes", "--encrypt"];
    for r in recipients {
        args.extend(["--recipient", r]);
    }
    Ok(run_gpg(&args, plain)?.0)
}

fn gpg_decrypt(cipher: &[u8]) -> Result<(Vec<u8>, Lock)> {
    let args = ["--batch", "--quiet", "--status-fd", "2", "--decrypt"];
    let (plain, stderr) = run_gpg(&args, cipher)?;
    let recipients = parse_enc_to(&stderr);
    if recipients.is_empty() {
        return Err("cannot find the gpg recipients of this profile"
            .to_string()
            .into());
    }
    Ok((plain, Lock::Gpg { recipients }))
}

/// Runs gpg with `input` on stdin. Returns (stdout, stderr) on success.
fn run_gpg(args: &[&str], input: &[u8]) -> Result<(Vec<u8>, String)> {
    let mut child = Command::new("gpg")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "gpg not found".to_string(),
            _ => format!("cannot run gpg: {e}"),
        })?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let out = std::thread::scope(|s| {
        // Write from a thread so a full pipe cannot deadlock; EPIPE is fine (gpg exited early).
        s.spawn(move || {
            let _ = stdin.write_all(input);
        });
        child.wait_with_output()
    })?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        return Err(format!("gpg failed: {}", gpg_message(&stderr)).into());
    }
    Ok((out.stdout, stderr))
}

fn gpg_message(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .filter(|l| !l.starts_with("[GNUPG:]"))
        .collect();
    // One line: the error printer escapes line feeds.
    lines.join("; ").trim().to_string()
}

/// A GPG secret key that can encrypt, as offered by `create --gpg` without KEY.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretKey {
    pub fingerprint: String,
    pub uid: String,
}

impl std::fmt::Display for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let short = self.fingerprint.len().saturating_sub(16);
        write!(f, "{} [{}]", self.uid, &self.fingerprint[short..])
    }
}

pub fn secret_keys() -> Result<Vec<SecretKey>> {
    let (out, _) = run_gpg(&["--batch", "--with-colons", "--list-secret-keys"], b"")?;
    Ok(parse_secret_keys(&String::from_utf8_lossy(&out)))
}

/// Parses `--with-colons` output: one block per `sec` record.
pub fn parse_secret_keys(colons: &str) -> Vec<SecretKey> {
    let mut blocks: Vec<Vec<Vec<&str>>> = Vec::new();
    for line in colons.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields[0] == "sec" {
            blocks.push(Vec::new());
        }
        if let Some(block) = blocks.last_mut() {
            block.push(fields);
        }
    }
    blocks.iter().filter_map(|b| usable_key(b)).collect()
}

fn field<'a>(fields: &[&'a str], i: usize) -> &'a str {
    fields.get(i).copied().unwrap_or("")
}

/// Field 2 = validity (e/r/d/i: expired, revoked, disabled, invalid); field 12 = capabilities,
/// where an uppercase `E` means the key as a whole can encrypt. First `fpr` = primary key.
fn usable_key(block: &[Vec<&str>]) -> Option<SecretKey> {
    let sec = &block[0];
    let valid = !matches!(field(sec, 1), "e" | "r" | "d" | "i");
    let record = |kind: &str| block.iter().find(|f| f[0] == kind).map(|f| field(f, 9));
    let fingerprint = record("fpr")?.to_string();
    let uid = record("uid").unwrap_or_default().to_string();
    (valid && field(sec, 11).contains('E')).then_some(SecretKey { fingerprint, uid })
}

pub fn parse_enc_to(stderr: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for line in stderr.lines() {
        let Some(rest) = line.strip_prefix("[GNUPG:] ENC_TO ") else {
            continue;
        };
        let id = rest.split_whitespace().next();
        if let Some(id) = id.filter(|id| !ids.iter().any(|i| i == id)) {
            ids.push(id.to_string());
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pass(s: &str) -> Lock {
        Lock::Passphrase(SecretString::from(s))
    }

    fn open(cipher: &[u8], p: &str) -> Result<(Vec<u8>, Lock)> {
        decrypt(cipher, || Ok(SecretString::from(p)))
    }

    // Real `gpg --list-secret-keys --with-colons` output (throwaway keys), plus a revoked key.
    const SECRET_KEYS: &str = "\
sec:u:255:22:226559A181480696:1795351498:::u:::scESC:::+::ed25519:::0:
fpr:::::::::717E27AC3AA440C6BB05F2E4226559A181480696:
uid:u::::1795351498::53AB29141270E65356D4200F2EB90D1F09DE2054::Alice <alice@example.invalid>::::::::::0:
ssb:u:255:18:AE9F15D1A843D155:1795351498::::::e:::+::cv25519::
fpr:::::::::1ABB1CD952E448B9B3B8C3ADAE9F15D1A843D155:
sec:u:255:22:FDF75656D44AB377:1795351498:::u:::scSC:::+::ed25519:::0:
fpr:::::::::68CED0D3BF01767940951C77FDF75656D44AB377:
uid:u::::1795351498::BB0F2D33372995AABC950DDADC6D40335C72BAC8::Signer <sign@example.invalid>::::::::::0:
sec:r:255:22:1111111111111111:1795351498:::u:::scESC:::+::ed25519:::0:
fpr:::::::::00000000000000000000000001111111111111111:
uid:r::::1795351498::00::Revoked <rev@example.invalid>::::::::::0:
";

    #[test]
    fn secret_keys_keep_only_usable_encryption_keys() {
        let keys = parse_secret_keys(SECRET_KEYS);
        assert_eq!(
            keys,
            [SecretKey {
                fingerprint: "717E27AC3AA440C6BB05F2E4226559A181480696".into(),
                uid: "Alice <alice@example.invalid>".into(),
            }]
        );
        assert_eq!(
            keys[0].to_string(),
            "Alice <alice@example.invalid> [226559A181480696]"
        );
        assert!(parse_secret_keys("").is_empty());
    }

    #[test]
    fn age_work_factor_is_fixed_not_calibrated() {
        let cipher = encrypt(&pass("correct horse"), b"x").unwrap();
        let stanza = cipher.split(|b| *b == b'\n').nth(1).unwrap();
        let stanza = String::from_utf8_lossy(stanza);
        assert!(
            stanza.starts_with("-> scrypt ") && stanza.ends_with(" 18"),
            "{stanza}"
        );
    }

    #[test]
    fn age_round_trip() {
        let cipher = encrypt(&pass("correct horse"), b"hello").unwrap();
        assert!(cipher.starts_with(b"age-encryption.org/v1\n"));
        let (plain, lock) = open(&cipher, "correct horse").unwrap();
        assert_eq!(plain, b"hello");
        assert!(matches!(lock, Lock::Passphrase(_)));
    }

    #[test]
    fn age_wrong_passphrase_errs() {
        let cipher = encrypt(&pass("correct horse"), b"hello").unwrap();
        assert!(open(&cipher, "wrong").is_err());
    }

    #[test]
    fn age_garbage_and_truncated_err() {
        let cipher = encrypt(&pass("correct horse"), b"hello").unwrap();
        for n in [22, 40, cipher.len() - 1] {
            assert!(open(&cipher[..n], "correct horse").is_err(), "cut at {n}");
        }
        let mut junk = b"age-encryption.org/v1\n".to_vec();
        junk.extend_from_slice(b"-> scrypt !!! x\nzzzz\n--- AAAA\n\xff\xfe");
        assert!(open(&junk, "x").is_err());
        assert!(open(AGE_MAGIC, "x").is_err());
    }

    #[test]
    fn age_error_does_not_echo_passphrase() {
        let cipher = encrypt(&pass("correct horse"), b"hello").unwrap();
        let msg = open(&cipher, "s3cretvalue").unwrap_err().to_string();
        assert!(!msg.contains("s3cretvalue"), "{msg}");
    }

    #[test]
    fn age_non_scrypt_file_is_rejected_without_prompt() {
        use age::x25519::Identity;
        let key = Identity::generate();
        let cipher = age::encrypt(&key.to_public(), b"x").unwrap();
        let err = decrypt(&cipher, || panic!("must not prompt")).unwrap_err();
        assert_eq!(err.to_string(), "only passphrase age files are supported");
    }

    #[test]
    fn gpg_encrypt_needs_a_recipient() {
        let err = encrypt(&Lock::Gpg { recipients: vec![] }, b"x").unwrap_err();
        assert_eq!(err.to_string(), "no gpg recipients");
    }

    #[test]
    fn gpg_message_drops_status_lines() {
        let s = "[GNUPG:] NO_SECKEY AB\ngpg: decryption failed: No secret key \n[GNUPG:] X\n";
        assert_eq!(gpg_message(s), "gpg: decryption failed: No secret key");
        let two = "gpg: k: skipped: No public key\n[GNUPG:] X\ngpg: encryption failed\n";
        assert_eq!(
            gpg_message(two),
            "gpg: k: skipped: No public key; gpg: encryption failed"
        );
    }

    #[test]
    fn enc_to_cases() {
        assert!(parse_enc_to("").is_empty());
        assert!(parse_enc_to("gpg: noise\n[GNUPG:] DECRYPTION_OKAY\n").is_empty());
        assert_eq!(parse_enc_to("[GNUPG:] ENC_TO AAAA 1 0\n"), ["AAAA"]);
        let two = "[GNUPG:] ENC_TO BBBB 1 0\n[GNUPG:] ENC_TO AAAA 18 0\n";
        assert_eq!(parse_enc_to(two), ["BBBB", "AAAA"]);
        let dup = "[GNUPG:] ENC_TO AAAA 1 0\ngpg: x\n[GNUPG:] ENC_TO BBBB 1 0\n[GNUPG:] ENC_TO AAAA 1 0\n";
        assert_eq!(parse_enc_to(dup), ["AAAA", "BBBB"]);
        assert!(parse_enc_to("[GNUPG:] ENC_TO\n").is_empty());
        assert!(parse_enc_to("[GNUPG:] ENC_TO \n").is_empty());
    }
}
