use age::secrecy::SecretString;

use crate::crypto::{self, Lock};
use crate::error::Result;
use crate::name::ProfileName;
use crate::store::Store;
use crate::vars::Vars;

pub struct Profile {
    pub name: ProfileName,
    pub lock: Lock,
    pub vars: Vars,
}

impl Profile {
    pub fn open(
        store: &Store,
        name: ProfileName,
        passphrase: impl FnOnce() -> Result<SecretString>,
    ) -> Result<Profile> {
        let (plain, lock) = crypto::decrypt(&store.read(&name)?, passphrase)?;
        // The serde error can quote payload text, so it is dropped.
        let vars = serde_json::from_slice::<Vars>(&plain)
            .map_err(|_| format!("profile \"{name}\" is corrupt"))?;
        Ok(Profile { name, lock, vars })
    }

    fn sealed(&self) -> Result<Vec<u8>> {
        crypto::encrypt(&self.lock, &serde_json::to_vec(&self.vars)?)
    }

    pub fn save(&self, store: &Store) -> Result<()> {
        store.replace(&self.name, &self.sealed()?)
    }

    pub fn create(&self, store: &Store) -> Result<()> {
        store.create(&self.name, &self.sealed()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vars::Var;

    const PASS: &str = "correct horse";

    fn secret() -> Result<SecretString> {
        Ok(SecretString::from(PASS))
    }

    fn fresh(t: &tempfile::TempDir) -> (Store, Profile) {
        let name: ProfileName = "Fresh DB1.TEST.LOCAL".parse().unwrap();
        let profile = Profile {
            name,
            lock: Lock::Passphrase(SecretString::from(PASS)),
            vars: Vars::default(),
        };
        (Store::new(t.path().join("store")), profile)
    }

    fn var(v: &str) -> Var {
        Var {
            value: v.into(),
            ..Var::default()
        }
    }

    #[test]
    fn create_open_modify_save_open() {
        let t = tempfile::tempdir().unwrap();
        let (store, mut p) = fresh(&t);
        p.create(&store).unwrap();
        assert!(p.create(&store).is_err());
        let mut q = Profile::open(&store, p.name.clone(), secret).unwrap();
        assert!(q.vars.0.is_empty());
        q.vars.0.insert("TOKEN".parse().unwrap(), var("v1"));
        q.save(&store).unwrap();
        p = Profile::open(&store, p.name, secret).unwrap();
        assert_eq!(
            p.vars.0[&"TOKEN".parse::<crate::name::VarName>().unwrap()],
            var("v1")
        );
        assert!(matches!(p.lock, Lock::Passphrase(_)));
    }

    #[test]
    fn open_wrong_passphrase_errs() {
        let t = tempfile::tempdir().unwrap();
        let (store, p) = fresh(&t);
        p.create(&store).unwrap();
        let r = Profile::open(&store, p.name, || Ok(SecretString::from("wrong")));
        assert!(r.is_err());
    }

    #[test]
    fn open_missing_errs() {
        let t = tempfile::tempdir().unwrap();
        let (store, p) = fresh(&t);
        let err = Profile::open(&store, p.name, secret).err().unwrap();
        assert_eq!(
            err.to_string(),
            "profile \"Fresh DB1.TEST.LOCAL\" does not exist"
        );
    }

    #[test]
    fn payload_has_var_but_not_passphrase() {
        let t = tempfile::tempdir().unwrap();
        let (store, mut p) = fresh(&t);
        p.vars.0.insert("TOKEN".parse().unwrap(), var("v1"));
        p.create(&store).unwrap();
        let (plain, _) = crypto::decrypt(&store.read(&p.name).unwrap(), secret).unwrap();
        let text = String::from_utf8(plain).unwrap();
        assert_eq!(text, r#"{"TOKEN":{"value":"v1"}}"#);
        assert!(!text.contains(PASS));
    }

    #[test]
    fn corrupt_payload_errs_without_echo() {
        let t = tempfile::tempdir().unwrap();
        let (store, p) = fresh(&t);
        let bad = crypto::encrypt(&p.lock, b"{not json TOPSECRET").unwrap();
        store.create(&p.name, &bad).unwrap();
        let err = Profile::open(&store, p.name, secret).err().unwrap();
        assert_eq!(
            err.to_string(),
            "profile \"Fresh DB1.TEST.LOCAL\" is corrupt"
        );
    }
}
