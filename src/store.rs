use std::ffi::OsString;
use std::io::Write;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::name::ProfileName;

pub struct Store {
    dir: PathBuf,
}

/// Empty values count as unset; a relative XDG_DATA_HOME is ignored (XDG spec).
pub fn resolve_dir(
    duckpond_dir: Option<OsString>,
    xdg_data_home: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf> {
    let set = |v: Option<OsString>| v.filter(|s| !s.is_empty()).map(PathBuf::from);
    if let Some(d) = set(duckpond_dir) {
        return Ok(d);
    }
    if let Some(x) = set(xdg_data_home).filter(|p| p.is_absolute()) {
        return Ok(x.join("duckpond"));
    }
    match set(home) {
        Some(h) => Ok(h.join(".local/share/duckpond")),
        None => Err("cannot find the profile store: set DUCKPOND_DIR or HOME"
            .to_string()
            .into()),
    }
}

/// Atomic write: temp file (0600) in the same directory, fsync, then rename.
pub fn write_atomic(path: &Path, bytes: &[u8], clobber: bool) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    if clobber {
        tmp.persist(path).map_err(|e| e.error)?;
        return Ok(());
    }
    tmp.persist_noclobber(path).map_err(|e| -> Error {
        match e.error.kind() {
            std::io::ErrorKind::AlreadyExists => {
                format!("{} already exists (use --force)", path.display()).into()
            }
            _ => e.error.into(),
        }
    })?;
    Ok(())
}

fn missing(name: &ProfileName) -> Error {
    format!("profile \"{name}\" does not exist").into()
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn from_env() -> Result<Self> {
        let var = std::env::var_os;
        let dir = resolve_dir(var("DUCKPOND_DIR"), var("XDG_DATA_HOME"), var("HOME"))?;
        Ok(Self::new(dir))
    }

    fn path(&self, name: &ProfileName) -> PathBuf {
        self.dir.join(format!("{name}.enc"))
    }

    pub fn list(&self) -> Result<Vec<ProfileName>> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry?;
            let file = entry.file_name();
            let stem = file.to_str().and_then(|f| f.strip_suffix(".enc"));
            if let Some(name) = stem.and_then(|s| s.parse::<ProfileName>().ok()) {
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }

    pub fn exists(&self, name: &ProfileName) -> bool {
        self.path(name).is_file()
    }

    pub fn read(&self, name: &ProfileName) -> Result<Vec<u8>> {
        std::fs::read(self.path(name)).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => missing(name),
            _ => e.into(),
        })
    }

    pub fn delete(&self, name: &ProfileName) -> Result<()> {
        std::fs::remove_file(self.path(name)).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => missing(name),
            _ => e.into(),
        })
    }

    pub fn create(&self, name: &ProfileName, bytes: &[u8]) -> Result<()> {
        if self.exists(name) {
            return Err(format!("profile \"{name}\" already exists").into());
        }
        self.write(name, bytes, false)
    }

    pub fn replace(&self, name: &ProfileName, bytes: &[u8]) -> Result<()> {
        self.write(name, bytes, true)
    }

    fn write(&self, name: &ProfileName, bytes: &[u8], clobber: bool) -> Result<()> {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.dir)?;
        write_atomic(&self.path(name), bytes, clobber)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn os(s: &str) -> Option<OsString> {
        Some(s.into())
    }

    fn pn(s: &str) -> ProfileName {
        s.parse().unwrap()
    }

    fn store(t: &tempfile::TempDir) -> Store {
        Store::new(t.path().join("store"))
    }

    fn mode(p: &Path) -> u32 {
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn resolve_dir_precedence() {
        let r = |d, x, h| resolve_dir(d, x, h).map(|p| p.to_string_lossy().into_owned());
        assert_eq!(r(os("/d"), os("/x"), os("/h")).unwrap(), "/d");
        assert_eq!(r(None, os("/x"), os("/h")).unwrap(), "/x/duckpond");
        assert_eq!(r(None, None, os("/h")).unwrap(), "/h/.local/share/duckpond");
        assert_eq!(
            r(os(""), os(""), os("/h")).unwrap(),
            "/h/.local/share/duckpond"
        );
        assert_eq!(
            r(None, os("rel"), os("/h")).unwrap(),
            "/h/.local/share/duckpond"
        );
        assert!(r(None, None, None).is_err());
        assert!(r(os(""), os("rel"), os("")).is_err());
    }

    #[test]
    fn list_filters_and_sorts() {
        let t = tempfile::tempdir().unwrap();
        let s = store(&t);
        assert!(s.list().unwrap().is_empty());
        std::fs::create_dir(&s.dir).unwrap();
        for f in [
            "b.enc",
            "Fresh DB1.TEST.LOCAL.enc",
            "a.enc",
            ".tmpXYZ",
            "x.txt",
            "..enc",
            "-bad.enc",
            "enc",
        ] {
            std::fs::write(s.dir.join(f), b"").unwrap();
        }
        let got: Vec<String> = s.list().unwrap().iter().map(|n| n.to_string()).collect();
        assert_eq!(got, ["Fresh DB1.TEST.LOCAL", "a", "b"]);
    }

    #[test]
    fn create_twice_keeps_first() {
        let t = tempfile::tempdir().unwrap();
        let s = store(&t);
        let n = pn("p");
        s.create(&n, b"one").unwrap();
        let err = s.create(&n, b"two").unwrap_err();
        assert_eq!(err.to_string(), "profile \"p\" already exists");
        assert_eq!(s.read(&n).unwrap(), b"one");
        assert!(s.exists(&n));
    }

    #[test]
    fn replace_overwrites_and_creates() {
        let t = tempfile::tempdir().unwrap();
        let s = store(&t);
        let n = pn("p");
        s.replace(&n, b"one").unwrap();
        s.replace(&n, b"two").unwrap();
        assert_eq!(s.read(&n).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(&s.dir).unwrap().count(), 1);
    }

    #[test]
    fn modes_are_private() {
        let t = tempfile::tempdir().unwrap();
        let s = Store::new(t.path().join("a/b/store"));
        s.create(&pn("p"), b"x").unwrap();
        assert_eq!(mode(&s.dir), 0o700);
        assert_eq!(mode(&t.path().join("a")), 0o700);
        assert_eq!(mode(&s.path(&pn("p"))), 0o600);
        s.replace(&pn("p"), b"y").unwrap();
        assert_eq!(mode(&s.path(&pn("p"))), 0o600);
    }

    #[test]
    fn read_and_delete_missing_err() {
        let t = tempfile::tempdir().unwrap();
        let s = store(&t);
        let want = "profile \"nope\" does not exist";
        assert_eq!(s.read(&pn("nope")).unwrap_err().to_string(), want);
        assert_eq!(s.delete(&pn("nope")).unwrap_err().to_string(), want);
        assert!(!s.exists(&pn("nope")));
    }

    #[test]
    fn delete_removes() {
        let t = tempfile::tempdir().unwrap();
        let s = store(&t);
        s.create(&pn("p"), b"x").unwrap();
        s.delete(&pn("p")).unwrap();
        assert!(!s.exists(&pn("p")));
    }

    #[test]
    fn write_atomic_no_clobber_message() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("out.env");
        write_atomic(&f, b"one", false).unwrap();
        let err = write_atomic(&f, b"two", false).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("{} already exists (use --force)", f.display())
        );
        assert_eq!(std::fs::read(&f).unwrap(), b"one");
        write_atomic(&f, b"two", true).unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(t.path()).unwrap().count(), 1);
    }
}
