use std::collections::BTreeMap;
use std::str::FromStr;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::name::VarName;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Var {
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<NaiveDate>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Vars(pub BTreeMap<VarName, Var>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expiry {
    On(NaiveDate),
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Sh,
    Fish,
}

impl FromStr for Expiry {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        if s == "none" {
            return Ok(Expiry::Never);
        }
        s.parse()
            .map(Expiry::On)
            .map_err(|_| format!("invalid expiry {s:?}: use YYYY-MM-DD or none").into())
    }
}

/// None keeps the field; an empty comment and `Expiry::Never` clear it.
fn apply_meta(var: &mut Var, comment: Option<String>, expires: Option<Expiry>) {
    if let Some(c) = comment {
        var.comment = Some(c).filter(|c| !c.is_empty());
    }
    if let Some(e) = expires {
        var.expires = match e {
            Expiry::On(d) => Some(d),
            Expiry::Never => None,
        };
    }
}

fn check_nul(name: &VarName, value: &str) -> Result<()> {
    if value.contains('\0') {
        return Err(format!("value of {name} contains a NUL byte").into());
    }
    Ok(())
}

impl Vars {
    pub fn set(
        &mut self,
        name: VarName,
        value: Option<String>,
        comment: Option<String>,
        expires: Option<Expiry>,
    ) -> Result<()> {
        let old = self.0.get(&name).cloned();
        let mut var = match (value, old) {
            (Some(value), old) => {
                check_nul(&name, &value)?;
                Var {
                    value,
                    ..old.unwrap_or_default()
                }
            }
            (None, Some(old)) => old,
            (None, None) => return Err(format!("variable {name} does not exist").into()),
        };
        apply_meta(&mut var, comment, expires);
        self.0.insert(name, var);
        Ok(())
    }

    pub fn unset(&mut self, names: &[VarName]) -> Result<()> {
        if let Some(missing) = names.iter().find(|n| !self.0.contains_key(n)) {
            return Err(format!("variable {missing} does not exist").into());
        }
        names.iter().for_each(|n| {
            self.0.remove(n);
        });
        Ok(())
    }

    pub fn merge(&mut self, pairs: Vec<(VarName, String)>) -> Result<()> {
        pairs.iter().try_for_each(|(n, v)| check_nul(n, v))?;
        for (name, value) in pairs {
            self.0.entry(name).or_default().value = value;
        }
        Ok(())
    }
}

fn parse_line(line: &str) -> Result<Option<(VarName, String)>> {
    let line = line.trim_start();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }
    let line = line.strip_prefix("export ").unwrap_or(line);
    let (name, value) = line.split_once('=').ok_or("missing '='".to_string())?;
    let name: VarName = name.parse()?;
    check_nul(&name, value)?;
    Ok(Some((name, value.to_string())))
}

pub fn parse_dotenv(text: &str) -> Result<Vec<(VarName, String)>> {
    let lines = text.lines().enumerate();
    let parsed = lines.filter_map(|(i, l)| {
        parse_line(l)
            .map_err(|e| Error::from(format!("line {}: {e}", i + 1)))
            .transpose()
    });
    parsed.collect()
}

pub fn to_dotenv(vars: &Vars, only: &[VarName]) -> Result<String> {
    if let Some(missing) = only.iter().find(|n| !vars.0.contains_key(n)) {
        return Err(format!("variable {missing} does not exist").into());
    }
    let mut out = String::new();
    for (name, var) in vars
        .0
        .iter()
        .filter(|(n, _)| only.is_empty() || only.contains(n))
    {
        // shortcut: raw format cannot hold CR/LF, upgrade = quoted dotenv
        if var.value.contains(['\n', '\r']) {
            return Err(
                format!("value of {name} contains a line break, dotenv cannot hold it").into(),
            );
        }
        out.push_str(&format!("{name}={}\n", var.value));
    }
    Ok(out)
}

fn quote(value: &str, shell: Shell) -> String {
    match shell {
        Shell::Sh => format!("'{}'", value.replace('\'', "'\\''")),
        Shell::Fish => format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'")),
    }
}

pub fn exports(vars: &Vars, shell: Shell) -> String {
    let line = |(n, v): (&VarName, &Var)| match shell {
        Shell::Sh => format!("export {n}={}\n", quote(&v.value, shell)),
        Shell::Fish => format!("set -gx {n} {}\n", quote(&v.value, shell)),
    };
    vars.0.iter().map(line).collect()
}

pub fn unsets(vars: &Vars, shell: Shell) -> String {
    let prefix = match shell {
        Shell::Sh => "unset",
        Shell::Fish => "set -e",
    };
    vars.0.keys().map(|n| format!("{prefix} {n}\n")).collect()
}

pub fn expired(vars: &Vars, today: NaiveDate) -> Vec<(&VarName, NaiveDate)> {
    let dated = vars.0.iter().filter_map(|(n, v)| Some((n, v.expires?)));
    dated.filter(|(_, d)| *d < today).collect()
}

fn escape(s: &str) -> String {
    let one = |c: char| match c {
        '\n' | '\r' | '\t' => c.escape_default().to_string(),
        c if c.is_control() => c.escape_unicode().to_string(),
        c => c.to_string(),
    };
    s.chars().map(one).collect()
}

fn expires_cell(var: &Var, today: NaiveDate) -> String {
    match var.expires {
        Some(d) if d < today => format!("{d} (expired)"),
        Some(d) => d.to_string(),
        None => String::new(),
    }
}

fn row(
    name: &VarName,
    var: &Var,
    reveal: bool,
    today: NaiveDate,
    cols: (bool, bool),
) -> Vec<String> {
    let value = if reveal {
        escape(&var.value)
    } else {
        "********".into()
    };
    let mut row = vec![name.to_string(), value];
    if cols.0 {
        row.push(expires_cell(var, today));
    }
    if cols.1 {
        row.push(escape(var.comment.as_deref().unwrap_or("")));
    }
    row
}

fn format_rows(rows: &[Vec<String>]) -> String {
    let width = |i: usize| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0);
    let widths: Vec<usize> = (0..rows[0].len()).map(width).collect();
    let mut out = String::new();
    for r in rows {
        let cells: Vec<String> = r
            .iter()
            .zip(&widths)
            .map(|(c, w)| format!("{c:<w$}"))
            .collect();
        out.push_str(cells.join("  ").trim_end());
        out.push('\n');
    }
    out
}

pub fn render_table(vars: &Vars, reveal: bool, today: NaiveDate) -> String {
    let cols = (
        vars.0.values().any(|v| v.expires.is_some()),
        vars.0.values().any(|v| v.comment.is_some()),
    );
    let mut header = vec!["NAME".to_string(), "VALUE".into()];
    if cols.0 {
        header.push("EXPIRES".into());
    }
    if cols.1 {
        header.push("COMMENT".into());
    }
    let mut rows = vec![header];
    rows.extend(vars.0.iter().map(|(n, v)| row(n, v, reveal, today, cols)));
    format_rows(&rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> VarName {
        s.parse().unwrap()
    }

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn vars(pairs: &[(&str, &str)]) -> Vars {
        let mut v = Vars::default();
        for (k, val) in pairs {
            v.set(n(k), Some(val.to_string()), None, None).unwrap();
        }
        v
    }

    fn err<T: std::fmt::Debug>(r: Result<T>) -> String {
        r.unwrap_err().to_string()
    }

    // ---- Expiry
    #[test]
    fn expiry_parses_date_and_none() {
        assert_eq!(
            "2026-01-31".parse::<Expiry>().unwrap(),
            Expiry::On(d("2026-01-31"))
        );
        assert_eq!("none".parse::<Expiry>().unwrap(), Expiry::Never);
        let msg = err("tomorrow".parse::<Expiry>());
        assert!(
            msg.contains("tomorrow") && msg.contains("YYYY-MM-DD"),
            "{msg}"
        );
    }

    // ---- serde
    #[test]
    fn payload_json_omits_none_and_round_trips() {
        let mut v = vars(&[("A", "1")]);
        v.set(
            n("B"),
            Some("2".into()),
            Some("note".into()),
            Some(Expiry::On(d("2030-05-06"))),
        )
        .unwrap();
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(
            json,
            r#"{"A":{"value":"1"},"B":{"value":"2","comment":"note","expires":"2030-05-06"}}"#
        );
        assert_eq!(serde_json::from_str::<Vars>(&json).unwrap(), v);
        assert!(serde_json::from_str::<Vars>(r#"{"bad name":{"value":"x"}}"#).is_err());
    }

    // ---- set
    #[test]
    fn set_creates_and_overwrites_keeping_meta() {
        let mut v = Vars::default();
        v.set(
            n("A"),
            Some("1".into()),
            Some("c".into()),
            Some(Expiry::On(d("2030-01-01"))),
        )
        .unwrap();
        v.set(n("A"), Some("2".into()), None, None).unwrap();
        let a = &v.0[&n("A")];
        assert_eq!(a.value, "2");
        assert_eq!(a.comment.as_deref(), Some("c"));
        assert_eq!(a.expires, Some(d("2030-01-01")));
    }

    #[test]
    fn set_comment_empty_clears_and_never_clears_expiry() {
        let mut v = Vars::default();
        v.set(
            n("A"),
            Some("1".into()),
            Some("c".into()),
            Some(Expiry::On(d("2030-01-01"))),
        )
        .unwrap();
        v.set(
            n("A"),
            Some("1".into()),
            Some(String::new()),
            Some(Expiry::Never),
        )
        .unwrap();
        let a = &v.0[&n("A")];
        assert_eq!((a.comment.clone(), a.expires), (None, None));
    }

    #[test]
    fn set_keep_value_changes_only_meta_and_needs_existing() {
        let mut v = vars(&[("A", "1")]);
        v.set(n("A"), None, Some("c".into()), None).unwrap();
        assert_eq!(v.0[&n("A")].value, "1");
        assert_eq!(v.0[&n("A")].comment.as_deref(), Some("c"));
        let before = v.clone();
        let msg = err(v.set(n("NOPE"), None, Some("c".into()), None));
        assert!(msg.contains("NOPE"), "{msg}");
        assert_eq!(v, before);
    }

    #[test]
    fn set_rejects_nul_naming_var_not_value() {
        let mut v = Vars::default();
        let msg = err(v.set(n("A"), Some("sec\0ret".into()), None, None));
        assert!(
            msg.contains('A') && msg.contains("NUL") && !msg.contains("sec"),
            "{msg}"
        );
        assert!(v.0.is_empty());
    }

    // ---- unset
    #[test]
    fn unset_removes_all_named() {
        let mut v = vars(&[("A", "1"), ("B", "2"), ("C", "3")]);
        v.unset(&[n("A"), n("C")]).unwrap();
        assert_eq!(v, vars(&[("B", "2")]));
    }

    #[test]
    fn unset_is_all_or_nothing() {
        let mut v = vars(&[("A", "1")]);
        let msg = err(v.unset(&[n("A"), n("MISSING")]));
        assert!(msg.contains("MISSING"), "{msg}");
        assert_eq!(v, vars(&[("A", "1")]));
    }

    // ---- merge
    #[test]
    fn merge_upserts_values_and_keeps_meta() {
        let mut v = Vars::default();
        v.set(n("A"), Some("1".into()), Some("c".into()), None)
            .unwrap();
        v.merge(vec![(n("A"), "9".into()), (n("B"), "2".into())])
            .unwrap();
        assert_eq!(v.0[&n("A")].value, "9");
        assert_eq!(v.0[&n("A")].comment.as_deref(), Some("c"));
        assert_eq!(v.0[&n("B")].value, "2");
    }

    #[test]
    fn merge_rejects_nul_and_changes_nothing() {
        let mut v = vars(&[("A", "1")]);
        let msg = err(v.merge(vec![(n("A"), "9".into()), (n("B"), "x\0y".into())]));
        assert!(msg.contains('B') && !msg.contains('x'), "{msg}");
        assert_eq!(v, vars(&[("A", "1")]));
    }

    // ---- dotenv
    #[test]
    fn dotenv_parse_rules() {
        let text = "# c\n\n  # indented\nA=1\nexport B=two words\nC=\nD=a=b\nE='q'\n  F=x";
        let got: Vec<(String, String)> = parse_dotenv(text)
            .unwrap()
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        let want: Vec<(String, String)> = [
            ("A", "1"),
            ("B", "two words"),
            ("C", ""),
            ("D", "a=b"),
            ("E", "'q'"),
            ("F", "x"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn dotenv_parse_strips_crlf() {
        let got = parse_dotenv("A=1\r\nB=2\r\n").unwrap();
        assert_eq!(got, vec![(n("A"), "1".into()), (n("B"), "2".into())]);
    }

    #[test]
    fn dotenv_parse_errors_have_line_numbers_and_no_values() {
        let msg = err(parse_dotenv("A=1\nnoequals"));
        assert!(msg.contains("line 2") && msg.contains('='), "{msg}");
        let msg = err(parse_dotenv("A=1\n\n1BAD=secret"));
        assert!(
            msg.contains("line 3") && msg.contains("1BAD") && !msg.contains("secret"),
            "{msg}"
        );
        let msg = err(parse_dotenv("A=se\0cret"));
        assert!(
            msg.contains("line 1") && msg.contains("NUL") && !msg.contains("se"),
            "{msg}"
        );
    }

    #[test]
    fn dotenv_to_dotenv_all_or_selected() {
        let v = vars(&[("B", "2"), ("A", "1")]);
        assert_eq!(to_dotenv(&v, &[]).unwrap(), "A=1\nB=2\n");
        assert_eq!(to_dotenv(&v, &[n("B")]).unwrap(), "B=2\n");
        let msg = err(to_dotenv(&v, &[n("ZZ")]));
        assert!(msg.contains("ZZ"), "{msg}");
    }

    #[test]
    fn dotenv_export_rejects_newline_naming_var() {
        for bad in ["x\ny", "x\r"] {
            let v = vars(&[("A", bad)]);
            let msg = err(to_dotenv(&v, &[]));
            assert!(msg.contains('A') && !msg.contains('x'), "{msg}");
        }
    }

    #[test]
    fn dotenv_round_trip_is_exact() {
        let v = vars(&[
            ("A", ""),
            ("B", "a=b"),
            ("C", "'q' \"d\" \\ $(x) #h"),
            ("D", " lead and trail "),
            ("E", "tab\t"),
        ]);
        let text = to_dotenv(&v, &[]).unwrap();
        let mut back = Vars::default();
        back.merge(parse_dotenv(&text).unwrap()).unwrap();
        assert_eq!(back, v);
    }

    // ---- shell
    #[test]
    fn sh_quoting() {
        let v = vars(&[
            ("A", "it's"),
            ("B", "a\\b"),
            ("C", "l1\nl2"),
            ("D", "$(x) `y` $Z"),
        ]);
        assert_eq!(
            exports(&v, Shell::Sh),
            "export A='it'\\''s'\nexport B='a\\b'\nexport C='l1\nl2'\nexport D='$(x) `y` $Z'\n"
        );
    }

    #[test]
    fn fish_quoting() {
        let v = vars(&[("A", "it's"), ("B", "a\\b"), ("C", "l1\nl2"), ("D", "$(x)")]);
        assert_eq!(
            exports(&v, Shell::Fish),
            "set -gx A 'it\\'s'\nset -gx B 'a\\\\b'\nset -gx C 'l1\nl2'\nset -gx D '$(x)'\n"
        );
    }

    #[test]
    fn unset_code() {
        let v = vars(&[("B", "2"), ("A", "1")]);
        assert_eq!(unsets(&v, Shell::Sh), "unset A\nunset B\n");
        assert_eq!(unsets(&v, Shell::Fish), "set -e A\nset -e B\n");
    }

    // ---- expired
    #[test]
    fn expired_is_strictly_before_today() {
        let mut v = Vars::default();
        for (k, date) in [
            ("OLD", "2025-01-01"),
            ("TODAY", "2026-06-01"),
            ("LATER", "2027-01-01"),
        ] {
            v.set(n(k), Some("x".into()), None, Some(Expiry::On(d(date))))
                .unwrap();
        }
        v.set(n("NONE"), Some("x".into()), None, None).unwrap();
        let got = expired(&v, d("2026-06-01"));
        assert_eq!(got, vec![(&n("OLD"), d("2025-01-01"))]);
    }

    // ---- table
    #[test]
    fn table_header_only_when_empty() {
        assert_eq!(
            render_table(&Vars::default(), false, d("2026-01-01")),
            "NAME  VALUE\n"
        );
    }

    #[test]
    fn table_masks_and_aligns() {
        let v = vars(&[("A", "1"), ("LONGER", "2")]);
        assert_eq!(
            render_table(&v, false, d("2026-01-01")),
            "NAME    VALUE\nA       ********\nLONGER  ********\n"
        );
        assert_eq!(
            render_table(&v, true, d("2026-01-01")),
            "NAME    VALUE\nA       1\nLONGER  2\n"
        );
    }

    #[test]
    fn table_optional_columns_and_expired_marker() {
        let mut v = Vars::default();
        v.set(
            n("A"),
            Some("1".into()),
            Some("hello".into()),
            Some(Expiry::On(d("2025-01-01"))),
        )
        .unwrap();
        v.set(
            n("B"),
            Some("2".into()),
            None,
            Some(Expiry::On(d("2030-01-01"))),
        )
        .unwrap();
        v.set(n("C"), Some("3".into()), None, None).unwrap();
        assert_eq!(
            render_table(&v, true, d("2026-01-01")),
            "NAME  VALUE  EXPIRES               COMMENT\n\
             A     1      2025-01-01 (expired)  hello\n\
             B     2      2030-01-01\n\
             C     3\n"
        );
    }

    #[test]
    fn table_only_comment_column() {
        let mut v = Vars::default();
        v.set(n("A"), Some("1".into()), Some("hi".into()), None)
            .unwrap();
        assert_eq!(
            render_table(&v, false, d("2026-01-01")),
            "NAME  VALUE     COMMENT\nA     ********  hi\n"
        );
    }

    #[test]
    fn table_escapes_control_chars() {
        let mut v = Vars::default();
        v.set(
            n("A"),
            Some("x\ny\u{1b}[31m".into()),
            Some("c\rd\te".into()),
            None,
        )
        .unwrap();
        let out = render_table(&v, true, d("2026-01-01"));
        assert!(
            out.contains("x\\ny\\u{1b}[31m") && out.contains("c\\rd\\te"),
            "{out}"
        );
        assert_eq!(out.lines().count(), 2);
        assert!(!out.contains('\u{1b}'));
    }
}
