//! Rust port of `xrpl/basics/BasicConfig.h`.
//!
//! The reference API mixes three roles:
//! - raw section storage,
//! - key/value lookup and typed parsing,
//! - legacy one-line section access.
//!
//! This Rust port preserves those roles while using explicit method names
//! where reference relied on overloading.

use crate::string_utilities::trim_whitespace;
use regex::Regex;
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::OnceLock;

pub type IniFileSections = HashMap<String, Vec<String>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyValueError {
    MultipleLines { section: String },
}

impl fmt::Display for LegacyValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MultipleLines { section } => write!(
                formatter,
                "A legacy value must have exactly one line. Section: {section}"
            ),
        }
    }
}

impl std::error::Error for LegacyValueError {}

#[derive(Debug, Clone, Default)]
pub struct Section {
    name: String,
    lookup: HashMap<String, String>,
    lines: Vec<String>,
    values: Vec<String>,
    had_trailing_comments: bool,
}

impl Section {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn values(&self) -> &[String] {
        &self.values
    }

    pub fn set_legacy(&mut self, value: impl Into<String>) {
        let value = value.into();
        if self.lines.is_empty() {
            self.lines.push(value);
        } else {
            self.lines[0] = value;
        }
    }

    pub fn legacy(&self) -> Result<String, LegacyValueError> {
        match self.lines.as_slice() {
            [] => Ok(String::new()),
            [line] => Ok(line.clone()),
            _ => Err(LegacyValueError::MultipleLines {
                section: self.name.clone(),
            }),
        }
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.lookup.insert(key.into(), value.into());
    }

    pub fn append_lines<I, S>(&mut self, lines: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let incoming = lines.into_iter().map(Into::into).collect::<Vec<_>>();
        self.lines.reserve(incoming.len());

        for mut line in incoming {
            if remove_comment(&mut line) && !line.is_empty() {
                self.had_trailing_comments = true;
            }

            if line.is_empty() {
                continue;
            }

            if let Some((key, value)) = parse_key_value(&line) {
                self.set(key, value);
            } else {
                self.values.push(line.clone());
            }

            self.lines.push(line);
        }
    }

    pub fn append(&mut self, line: impl Into<String>) {
        self.append_lines([line.into()]);
    }

    pub fn exists(&self, name: &str) -> bool {
        self.lookup.contains_key(name)
    }

    pub fn get<T>(&self, name: &str) -> Result<Option<T>, T::Err>
    where
        T: FromStr,
    {
        match self.lookup.get(name) {
            Some(value) => T::from_str(value).map(Some),
            None => Ok(None),
        }
    }

    pub fn value_or<T>(&self, name: &str, other: T) -> Result<T, T::Err>
    where
        T: FromStr,
    {
        match self.get(name)? {
            Some(value) => Ok(value),
            None => Ok(other),
        }
    }

    pub fn had_trailing_comments(&self) -> bool {
        self.had_trailing_comments
    }

    pub fn empty(&self) -> bool {
        self.lookup.is_empty()
    }

    pub fn size(&self) -> usize {
        self.lookup.len()
    }

    pub fn iter(&self) -> std::collections::hash_map::Iter<'_, String, String> {
        self.lookup.iter()
    }
}

impl fmt::Display for Section {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (key, value) in &self.lookup {
            writeln!(formatter, "{key}={value}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct BasicConfig {
    map: HashMap<String, Section>,
}

impl BasicConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn exists(&self, name: &str) -> bool {
        self.map.contains_key(name)
    }

    pub fn section(&self, name: &str) -> &Section {
        if let Some(section) = self.map.get(name) {
            section
        } else {
            empty_section()
        }
    }

    pub fn section_mut(&mut self, name: impl Into<String>) -> &mut Section {
        let name = name.into();
        self.map
            .entry(name.clone())
            .or_insert_with(|| Section::new(name))
    }

    pub fn overwrite(
        &mut self,
        section: impl Into<String>,
        key: impl Into<String>,
        value: impl Into<String>,
    ) {
        self.section_mut(section).set(key, value);
    }

    pub fn deprecated_clear_section(&mut self, section: &str) {
        if self.map.contains_key(section) {
            self.map.insert(section.to_owned(), Section::new(section));
        }
    }

    pub fn set_legacy(&mut self, section: impl Into<String>, value: impl Into<String>) {
        self.section_mut(section).set_legacy(value);
    }

    pub fn legacy(&self, section_name: &str) -> Result<String, LegacyValueError> {
        self.section(section_name).legacy()
    }

    pub fn had_trailing_comments(&self) -> bool {
        self.map.values().any(Section::had_trailing_comments)
    }

    pub fn build(&mut self, ini_sections: &IniFileSections) {
        for (name, lines) in ini_sections {
            self.section_mut(name.clone()).append_lines(lines.clone());
        }
    }

    pub fn iter(&self) -> std::collections::hash_map::Iter<'_, String, Section> {
        self.map.iter()
    }
}

impl fmt::Display for BasicConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (name, section) in &self.map {
            write!(formatter, "[{name}]\n{section}")?;
        }
        Ok(())
    }
}

pub fn set<T>(target: &mut T, name: &str, section: &Section) -> bool
where
    T: FromStr,
{
    match section.get::<T>(name) {
        Ok(Some(value)) => {
            *target = value;
            true
        }
        Ok(None) | Err(_) => false,
    }
}

pub fn set_with_default<T>(target: &mut T, default_value: T, name: &str, section: &Section) -> bool
where
    T: FromStr + Clone,
{
    let found_and_valid = set(target, name, section);
    if !found_and_valid {
        *target = default_value;
    }
    found_and_valid
}

pub fn get<T>(section: &Section, name: &str, default_value: T) -> T
where
    T: FromStr,
{
    match section.get(name) {
        Ok(Some(value)) => value,
        Ok(None) => default_value,
        Err(_) => default_value,
    }
}

pub fn get_string(section: &Section, name: &str, default_value: &str) -> String {
    match section.get::<String>(name) {
        Ok(Some(value)) => value,
        Ok(None) | Err(_) => default_value.to_owned(),
    }
}

pub fn get_if_exists<T>(section: &Section, name: &str, target: &mut T) -> bool
where
    T: FromStr,
{
    set(target, name, section)
}

pub fn get_if_exists_bool(section: &Section, name: &str, target: &mut bool) -> bool {
    let mut int_value = 0i32;
    let found = get_if_exists(section, name, &mut int_value);
    if found {
        *target = int_value != 0;
    }
    found
}

fn empty_section() -> &'static Section {
    static EMPTY: OnceLock<Section> = OnceLock::new();
    EMPTY.get_or_init(|| Section::new(""))
}

fn key_value_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^\s*([a-zA-Z][_a-zA-Z0-9]*)\s*=\s*(.*\S+)\s*$")
            .expect("BasicConfig key/value regex should compile")
    })
}

fn parse_key_value(line: &str) -> Option<(String, String)> {
    let captures = key_value_regex().captures(line)?;
    Some((
        captures.get(1)?.as_str().to_owned(),
        captures.get(2)?.as_str().to_owned(),
    ))
}

fn remove_comment(value: &mut String) -> bool {
    let mut removed_trailing = false;
    let mut search_from = 0usize;

    while search_from <= value.len() {
        let Some(relative_index) = value[search_from..].find('#') else {
            break;
        };
        let comment = search_from + relative_index;

        if comment == 0 {
            value.clear();
            break;
        }

        if value.as_bytes()[comment - 1] == b'\\' {
            value.remove(comment - 1);
            search_from = comment;
            continue;
        }

        *value = trim_whitespace(value[..comment].to_owned());
        removed_trailing = true;
        break;
    }

    removed_trailing
}

#[cfg(test)]
mod tests {
    use super::{
        BasicConfig, IniFileSections, LegacyValueError, Section, get, get_if_exists,
        get_if_exists_bool, get_string, set, set_with_default,
    };

    #[test]
    fn append_parses_key_values_values_and_comments() {
        let mut section = Section::new("test");
        section.append_lines([
            "alpha = one",
            "beta = two # trailing",
            "value line",
            "escaped\\#hash",
            "# full comment",
            "gamma = three\\#kept # stripped",
            "empty =    ",
        ]);

        assert_eq!(section.lines().len(), 6);
        assert_eq!(section.lines()[0], "alpha = one");
        assert_eq!(section.lines()[1], "beta = two");
        assert_eq!(section.lines()[2], "value line");
        assert_eq!(section.lines()[3], "escaped#hash");
        assert_eq!(section.lines()[4], "gamma = three#kept");
        assert_eq!(section.lines()[5], "empty =    ");

        assert_eq!(
            section.values(),
            &["value line", "escaped#hash", "empty =    "]
        );
        assert!(section.had_trailing_comments());
        assert!(section.exists("alpha"));
        assert!(section.exists("beta"));
        assert!(section.exists("gamma"));
        assert!(!section.exists("empty"));
        assert_eq!(
            section.get::<String>("alpha").unwrap(),
            Some("one".to_owned())
        );
        assert_eq!(
            section.get::<String>("beta").unwrap(),
            Some("two".to_owned())
        );
        assert_eq!(
            section.get::<String>("gamma").unwrap(),
            Some("three#kept".to_owned())
        );
    }

    #[test]
    fn legacy_value_requires_exactly_one_line() {
        let mut section = Section::new("legacy");
        assert_eq!(section.legacy().unwrap(), "");

        section.set_legacy("single");
        assert_eq!(section.legacy().unwrap(), "single");

        section.append("second");
        assert_eq!(
            section.legacy().unwrap_err(),
            LegacyValueError::MultipleLines {
                section: "legacy".to_owned()
            }
        );
    }

    #[test]
    fn set_only_updates_lookup_not_lines() {
        let mut section = Section::new("lookup");
        section.set("path", "/tmp/db");
        section.set("count", "42");

        assert_eq!(section.size(), 2);
        assert!(section.lines().is_empty());
        assert!(section.values().is_empty());
    }

    #[test]
    fn helper_functions_match_cpp_role() {
        let mut section = Section::new("helpers");
        section.set("threads", "16");
        section.set("enabled", "1");
        section.set("broken", "abc");

        let mut threads = 0usize;
        assert!(set(&mut threads, "threads", &section));
        assert_eq!(threads, 16);

        assert!(!set(&mut threads, "broken", &section));
        assert_eq!(threads, 16);

        assert!(!set_with_default(&mut threads, 8, "missing", &section));
        assert_eq!(threads, 8);

        assert_eq!(get(&section, "threads", 4usize), 16);
        assert_eq!(get(&section, "broken", 4usize), 4);
        assert_eq!(get_string(&section, "missing", "sqlite"), "sqlite");

        let mut enabled = false;
        assert!(get_if_exists_bool(&section, "enabled", &mut enabled));
        assert!(enabled);

        let mut parsed = 0usize;
        assert!(get_if_exists(&section, "threads", &mut parsed));
        assert_eq!(parsed, 16);
    }

    #[test]
    fn basic_config_build_overwrite_and_clear_match_cpp_role() {
        let mut config = BasicConfig::new();
        let mut sections = IniFileSections::new();
        sections.insert(
            "server".to_owned(),
            vec!["port = 51234".to_owned(), "admin".to_owned()],
        );
        sections.insert(
            "database_path".to_owned(),
            vec!["/var/lib/xrpld".to_owned()],
        );

        config.build(&sections);

        assert!(config.exists("server"));
        assert_eq!(
            config.section("server").get::<u16>("port").unwrap(),
            Some(51234)
        );
        assert_eq!(config.section("server").values(), &["admin"]);
        assert_eq!(config.legacy("database_path").unwrap(), "/var/lib/xrpld");

        config.overwrite("server", "ip", "127.0.0.1");
        assert_eq!(
            config.section("server").get::<String>("ip").unwrap(),
            Some("127.0.0.1".to_owned())
        );

        config.deprecated_clear_section("server");
        assert!(config.section("server").empty());
        assert!(config.section("server").lines().is_empty());
        assert!(!config.section("server").had_trailing_comments());
    }

    #[test]
    fn missing_section_returns_shared_empty_view() {
        let config = BasicConfig::new();
        let section = config.section("missing");
        assert_eq!(section.name(), "");
        assert!(section.empty());
        assert!(section.lines().is_empty());
        assert!(section.values().is_empty());
    }
}
