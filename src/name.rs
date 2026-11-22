use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::Error;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileName(String);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct VarName(String);

impl FromStr for ProfileName {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        let ok_char = |c: char| c.is_ascii_alphanumeric() || " ._@+-".contains(c);
        let ok = (1..=64).contains(&s.len())
            && s.chars().all(ok_char)
            && s.starts_with(|c: char| c.is_ascii_alphanumeric())
            && !s.ends_with(' ');
        if ok {
            return Ok(Self(s.to_string()));
        }
        Err(format!(
            "invalid profile name {s:?}: use 1-64 characters from A-Z a-z 0-9 space . _ @ + -, \
             starting with a letter or digit and not ending with a space"
        )
        .into())
    }
}

impl TryFrom<String> for VarName {
    type Error = Error;
    fn try_from(s: String) -> Result<Self, Error> {
        let mut chars = s.chars();
        let first = chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
        if first && s.len() <= 128 && chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Ok(Self(s));
        }
        Err(format!(
            "invalid variable name {s:?}: use at most 128 characters matching [A-Za-z_][A-Za-z0-9_]*"
        )
        .into())
    }
}

impl FromStr for VarName {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        s.to_string().try_into()
    }
}

macro_rules! text_impls {
    ($t:ty) => {
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl AsRef<str> for $t {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}
text_impls!(ProfileName);
text_impls!(VarName);

#[cfg(test)]
mod tests {
    use super::*;

    fn bad_profile(s: &str) -> String {
        s.parse::<ProfileName>().unwrap_err().to_string()
    }

    #[test]
    fn profile_accepts_real_names() {
        for s in [
            "Fresh DB1.TEST.LOCAL",
            "Ansible vault password",
            "Openly",
            "a",
            "9x",
            "u@h+x_y-z",
        ] {
            assert_eq!(s.parse::<ProfileName>().unwrap().as_ref(), s);
        }
    }

    #[test]
    fn profile_length_limit() {
        assert!("a".repeat(64).parse::<ProfileName>().is_ok());
        assert!("a".repeat(65).parse::<ProfileName>().is_err());
    }

    #[test]
    fn profile_rejects_bad_names_and_names_input_and_rule() {
        for s in [
            "", "../x", "a/b", ".hidden", "-x", " x", "x ", "a\\b", "a\nb", "é", "a\0b",
        ] {
            let msg = bad_profile(s);
            assert!(msg.contains("profile name"), "{msg}");
            assert!(msg.contains(&format!("{s:?}")), "{msg}");
        }
    }

    #[test]
    fn var_accepts_and_limits() {
        assert!("_A1".parse::<VarName>().is_ok());
        assert!("a".repeat(128).parse::<VarName>().is_ok());
        assert!("a".repeat(129).parse::<VarName>().is_err());
    }

    #[test]
    fn var_rejects_bad_names() {
        for s in ["", "1A", "A-B", "A B", "A=B", "Aé", "A\n"] {
            let msg = s.parse::<VarName>().unwrap_err().to_string();
            assert!(
                msg.contains("variable name") && msg.contains(&format!("{s:?}")),
                "{msg}"
            );
        }
    }

    #[test]
    fn var_name_serde() {
        let n: VarName = serde_json::from_str("\"FOO\"").unwrap();
        assert_eq!(serde_json::to_string(&n).unwrap(), "\"FOO\"");
        assert!(serde_json::from_str::<VarName>("\"1x\"").is_err());
        let m: std::collections::BTreeMap<VarName, u8> = serde_json::from_str("{\"A\":1}").unwrap();
        assert_eq!(serde_json::to_string(&m).unwrap(), "{\"A\":1}");
        assert!(
            serde_json::from_str::<std::collections::BTreeMap<VarName, u8>>("{\"a b\":1}").is_err()
        );
    }
}
