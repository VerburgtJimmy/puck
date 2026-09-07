//! Surgical `composer.json` edits matching Composer `JsonManipulator`.
//!
//! Preserves surrounding formatting; rewrites only the touched require map
//! (or inserts a new top-level key). Platform-aware `sort-packages` follows
//! `Composer\Json\JsonManipulator::sortPackages` +
//! `PlatformRepository::isPlatformPackage`.

use crate::{Error, Result};
use serde_json::{Map, Value};
use std::cmp::Ordering;

/// Detected whitespace style for a composer.json document.
#[derive(Debug, Clone)]
pub struct JsonManipulator {
    contents: String,
    newline: String,
    indent: String,
}

impl JsonManipulator {
    pub fn new(contents: &str) -> Result<Self> {
        let trimmed = contents.trim();
        let contents = if trimmed.is_empty() {
            "{}".to_string()
        } else {
            trimmed.to_string()
        };
        if !contents.starts_with('{') || !contents.ends_with('}') {
            return Err(Error::Parse("The json file must be an object ({})".into()));
        }
        let newline = if contents.contains("\r\n") {
            "\r\n".to_string()
        } else {
            "\n".to_string()
        };
        let contents = if contents == "{}" {
            format!("{{{newline}}}")
        } else {
            contents
        };
        let indent = detect_indenting(&contents);
        Ok(Self {
            contents,
            newline,
            indent,
        })
    }

    pub fn get_contents(&self) -> String {
        format!("{}{}", self.contents, self.newline)
    }

    /// Add or replace a link in `require` / `require-dev` (etc.).
    pub fn add_link(
        &mut self,
        link_type: &str,
        package: &str,
        constraint: &str,
        sort_packages: bool,
    ) -> Result<()> {
        let decoded: Value =
            serde_json::from_str(&self.contents).map_err(|e| Error::Parse(e.to_string()))?;
        let Value::Object(root) = &decoded else {
            return Err(Error::RootNotObject);
        };

        if !root.contains_key(link_type) {
            let mut map = Map::new();
            map.insert(package.to_string(), Value::String(constraint.to_string()));
            return self.add_main_key(link_type, Value::Object(map));
        }

        let Some(span) = find_top_level_object_value_span(&self.contents, link_type)? else {
            return Err(Error::Parse(format!(
                "could not locate {link_type} object in composer.json"
            )));
        };

        let links_text = &self.contents[span.start..span.end];
        let mut links: Map<String, Value> = serde_json::from_str(links_text)
            .map_err(|e| Error::Parse(format!("parse {link_type}: {e}")))?;

        let already = links.contains_key(package);
        links.insert(package.to_string(), Value::String(constraint.to_string()));

        let new_links = if sort_packages {
            sort_packages_map(&mut links);
            self.format_object(&links, 0)
        } else if already {
            // Constraint-only update: rewrite the whole object to keep key order.
            self.format_object(&links, 0)
        } else {
            // Append before closing brace (Composer JsonManipulator).
            append_package_to_object_text(
                links_text,
                package,
                constraint,
                &self.newline,
                &self.indent,
            )?
        };

        self.contents = format!(
            "{}{}{}",
            &self.contents[..span.start],
            new_links,
            &self.contents[span.end..]
        );
        Ok(())
    }

    /// Remove a package from `require` and/or `require-dev`.
    ///
    /// Returns whether the name was present in at least one map.
    pub fn remove_link(&mut self, package: &str) -> Result<bool> {
        let mut removed = false;
        for link_type in ["require", "require-dev"] {
            if self.remove_sub_node(link_type, package)? {
                removed = true;
            }
        }
        Ok(removed)
    }

    fn remove_sub_node(&mut self, main_node: &str, name: &str) -> Result<bool> {
        let decoded: Value =
            serde_json::from_str(&self.contents).map_err(|e| Error::Parse(e.to_string()))?;
        let Some(Value::Object(map)) = decoded.get(main_node) else {
            return Ok(false);
        };
        if map.is_empty() {
            return Ok(true);
        }
        // Case-insensitive match on existing key (Composer uses regex /i).
        let existing = map.keys().find(|k| k.eq_ignore_ascii_case(name)).cloned();
        let Some(existing) = existing else {
            return Ok(false);
        };

        let Some(span) = find_top_level_object_value_span(&self.contents, main_node)? else {
            return Err(Error::Parse(format!(
                "could not locate {main_node} object in composer.json"
            )));
        };
        let links_text = &self.contents[span.start..span.end];
        let mut links: Map<String, Value> = serde_json::from_str(links_text)
            .map_err(|e| Error::Parse(format!("parse {main_node}: {e}")))?;
        if links.shift_remove(&existing).is_none() {
            return Ok(false);
        }
        let new_links = if links.is_empty() {
            format!("{{{}{}}}", self.newline, self.indent)
        } else {
            self.format_object(&links, 0)
        };
        self.contents = format!(
            "{}{}{}",
            &self.contents[..span.start],
            new_links,
            &self.contents[span.end..]
        );
        Ok(true)
    }

    fn add_main_key(&mut self, key: &str, content: Value) -> Result<()> {
        let formatted = self.format_value(&content, 0);
        let key_enc = json_encode_string(key);
        // Insert before final `}` of the root object.
        let Some(close) = self.contents.rfind('}') else {
            return Err(Error::Parse("missing root closing brace".into()));
        };
        let empty_root = self.contents[1..close].trim().is_empty();
        if empty_root {
            self.contents = format!(
                "{{{}{}{}: {}{}{}",
                self.newline, self.indent, key_enc, formatted, self.newline, '}'
            );
            return Ok(());
        }
        let before = self.contents[..close]
            .trim_end_matches([' ', '\t'])
            .trim_end_matches('\n')
            .trim_end_matches('\r')
            .to_string();
        self.contents = format!(
            "{},{}{}{}: {}{}{}",
            before, self.newline, self.indent, key_enc, formatted, self.newline, '}'
        );
        Ok(())
    }

    fn format_object(&self, map: &Map<String, Value>, depth: usize) -> String {
        if map.is_empty() {
            return format!("{{{}{}}}", self.newline, self.indent.repeat(depth + 1));
        }
        let mut elems = Vec::with_capacity(map.len());
        for (k, v) in map {
            elems.push(format!(
                "{}{}: {}",
                self.indent.repeat(depth + 2),
                json_encode_string(k),
                self.format_value(v, depth + 1)
            ));
        }
        format!(
            "{{{}{}{}{}{}",
            self.newline,
            elems.join(&format!(",{}", self.newline)),
            self.newline,
            self.indent.repeat(depth + 1),
            '}'
        )
    }

    fn format_value(&self, data: &Value, depth: usize) -> String {
        match data {
            Value::Object(map) => self.format_object(map, depth),
            Value::Array(items) => {
                if items.is_empty() {
                    return "[]".into();
                }
                let parts: Vec<String> = items
                    .iter()
                    .map(|v| self.format_value(v, depth + 1))
                    .collect();
                format!("[{}]", parts.join(", "))
            }
            Value::String(s) => json_encode_string(s),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            Value::Null => "null".into(),
        }
    }
}

/// Composer `PlatformRepository::PLATFORM_PACKAGE_REGEX` (subset used for sort).
pub fn is_platform_package(name: &str) -> bool {
    // {^(?:php(?:-64bit|-ipv6|-zts|-debug)?|hhvm|(?:ext|lib)-[a-z0-9](?:[_.-]?[a-z0-9]+)*|composer(?:-(?:plugin|runtime)-api)?)$}iD
    let name = name.to_ascii_lowercase();
    if name == "php"
        || name == "php-64bit"
        || name == "php-ipv6"
        || name == "php-zts"
        || name == "php-debug"
        || name == "hhvm"
        || name == "composer"
        || name == "composer-plugin-api"
        || name == "composer-runtime-api"
    {
        return true;
    }
    platform_ext_or_lib(&name)
}

fn platform_ext_or_lib(name: &str) -> bool {
    let rest = if let Some(r) = name.strip_prefix("ext-") {
        r
    } else if let Some(r) = name.strip_prefix("lib-") {
        r
    } else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    let bytes = rest.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() {
        return false;
    }
    let mut i = 1;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_alphanumeric() {
            i += 1;
            continue;
        }
        if matches!(b, b'_' | b'.' | b'-') {
            i += 1;
            if i >= bytes.len() || !bytes[i].is_ascii_alphanumeric() {
                return false;
            }
            i += 1;
            continue;
        }
        return false;
    }
    true
}

/// Sort require-map keys like Composer `JsonManipulator::sortPackages`.
pub fn sort_packages_map(packages: &mut Map<String, Value>) {
    let old = std::mem::take(packages);
    let mut entries: Vec<(String, Value)> = old.into_iter().collect();
    entries.sort_by(|a, b| nat_cmp(&sort_prefix(&a.0), &sort_prefix(&b.0)));
    for (k, v) in entries {
        packages.insert(k, v);
    }
}

fn sort_prefix(requirement: &str) -> String {
    if is_platform_package(requirement) {
        let lower = requirement.to_ascii_lowercase();
        if lower.starts_with("php") {
            return format!("0-{requirement}");
        }
        if lower.starts_with("hhvm") {
            return format!("1-{requirement}");
        }
        if lower.starts_with("ext") {
            return format!("2-{requirement}");
        }
        if lower.starts_with("lib") {
            return format!("3-{requirement}");
        }
        // /^\D/ → 4- (composer, composer-plugin-api, …)
        return format!("4-{requirement}");
    }
    format!("5-{requirement}")
}

/// PHP `strnatcmp`-ish: digit runs compared numerically, else byte-wise.
fn nat_cmp(a: &str, b: &str) -> Ordering {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    let mut i = 0;
    let mut j = 0;
    while i < ab.len() && j < bb.len() {
        if ab[i].is_ascii_digit() && bb[j].is_ascii_digit() {
            while i < ab.len() && ab[i] == b'0' {
                i += 1;
            }
            while j < bb.len() && bb[j] == b'0' {
                j += 1;
            }
            let i0 = i;
            let j0 = j;
            while i < ab.len() && ab[i].is_ascii_digit() {
                i += 1;
            }
            while j < bb.len() && bb[j].is_ascii_digit() {
                j += 1;
            }
            let da = &ab[i0..i];
            let db = &bb[j0..j];
            match da.len().cmp(&db.len()) {
                Ordering::Equal => {}
                non_eq => return non_eq,
            }
            match da.cmp(db) {
                Ordering::Equal => {}
                non_eq => return non_eq,
            }
            continue;
        }
        match ab[i].cmp(&bb[j]) {
            Ordering::Equal => {
                i += 1;
                j += 1;
            }
            non_eq => return non_eq,
        }
    }
    ab.len().cmp(&bb.len())
}

fn detect_indenting(json: &str) -> String {
    for line in json.lines() {
        let rest = line.trim_start_matches([' ', '\t']);
        if rest.starts_with('"') {
            let indent_len = line.len() - rest.len();
            if indent_len > 0 {
                return line[..indent_len].to_string();
            }
        }
    }
    "    ".to_string()
}

fn json_encode_string(s: &str) -> String {
    // Composer JsonFile::encode uses JSON_UNESCAPED_SLASHES | UNESCAPED_UNICODE.
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[derive(Debug, Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
}

/// Locate the value span of a top-level object key whose value is an object `{…}`.
fn find_top_level_object_value_span(contents: &str, key: &str) -> Result<Option<Span>> {
    let bytes = contents.as_bytes();
    let mut i = 0;
    // Skip BOM / leading space to `{`
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b'{' {
        return Err(Error::Parse("root must be an object".into()));
    }
    i += 1;
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            return Ok(None);
        }
        if bytes[i] == b'}' {
            return Ok(None);
        }
        if bytes[i] != b'"' {
            return Err(Error::Parse("expected string key in object".into()));
        }
        let (key_str, next) = parse_json_string(contents, i)?;
        i = next;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b':' {
            return Err(Error::Parse("expected ':' after key".into()));
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let value_start = i;
        let value_end = skip_json_value(contents, i)?;
        if key_str == key {
            // Value must be object for require maps.
            if contents.as_bytes().get(value_start) == Some(&b'{') {
                return Ok(Some(Span {
                    start: value_start,
                    end: value_end,
                }));
            }
            return Err(Error::Parse(format!("{key} must be an object")));
        }
        i = value_end;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b',' {
            i += 1;
            continue;
        }
        if i < bytes.len() && bytes[i] == b'}' {
            return Ok(None);
        }
    }
}

fn parse_json_string(contents: &str, start: usize) -> Result<(String, usize)> {
    let bytes = contents.as_bytes();
    if bytes.get(start) != Some(&b'"') {
        return Err(Error::Parse("expected string".into()));
    }
    let mut i = start + 1;
    let mut out = String::new();
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return Ok((out, i + 1)),
            b'\\' => {
                i += 1;
                if i >= bytes.len() {
                    break;
                }
                match bytes[i] {
                    b'"' | b'\\' | b'/' => out.push(bytes[i] as char),
                    b'b' => out.push('\u{08}'),
                    b'f' => out.push('\u{0c}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        if i + 4 >= bytes.len() {
                            return Err(Error::Parse("bad unicode escape".into()));
                        }
                        let hex = &contents[i + 1..i + 5];
                        let code = u32::from_str_radix(hex, 16)
                            .map_err(|_| Error::Parse("bad unicode escape".into()))?;
                        out.push(
                            char::from_u32(code)
                                .ok_or_else(|| Error::Parse("bad unicode escape".into()))?,
                        );
                        i += 4;
                    }
                    _ => return Err(Error::Parse("bad escape".into())),
                }
                i += 1;
            }
            c => {
                out.push(c as char);
                i += 1;
            }
        }
    }
    Err(Error::Parse("unterminated string".into()))
}

fn skip_json_value(contents: &str, start: usize) -> Result<usize> {
    let bytes = contents.as_bytes();
    let mut i = start;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() {
        return Err(Error::Parse("unexpected end of json".into()));
    }
    match bytes[i] {
        b'"' => {
            let (_, end) = parse_json_string(contents, i)?;
            Ok(end)
        }
        b'{' | b'[' => {
            let open = bytes[i];
            let close = if open == b'{' { b'}' } else { b']' };
            i += 1;
            let mut depth = 1;
            while i < bytes.len() {
                match bytes[i] {
                    b'"' => {
                        let (_, end) = parse_json_string(contents, i)?;
                        i = end;
                        continue;
                    }
                    b if b == open => depth += 1,
                    b if b == close => {
                        depth -= 1;
                        if depth == 0 {
                            return Ok(i + 1);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            Err(Error::Parse("unbalanced brackets".into()))
        }
        b't' if contents[i..].starts_with("true") => Ok(i + 4),
        b'f' if contents[i..].starts_with("false") => Ok(i + 5),
        b'n' if contents[i..].starts_with("null") => Ok(i + 4),
        b'-' | b'0'..=b'9' => {
            i += 1;
            while i < bytes.len()
                && matches!(bytes[i], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
            {
                i += 1;
            }
            Ok(i)
        }
        _ => Err(Error::Parse(format!("unexpected json value at byte {i}"))),
    }
}

fn append_package_to_object_text(
    links_text: &str,
    package: &str,
    constraint: &str,
    newline: &str,
    indent: &str,
) -> Result<String> {
    // Match Composer: if object has content, insert `, newline indent indent "pkg": "c" ` before trailing `}`.
    let trimmed = links_text.trim();
    if trimmed == "{}" || trimmed == format!("{{{newline}}}") {
        return Ok(format!(
            "{{{}{}{}{}: {}{}{}{}",
            newline,
            indent,
            indent,
            json_encode_string(package),
            json_encode_string(constraint),
            newline,
            indent,
            '}'
        ));
    }
    let Some(close) = links_text.rfind('}') else {
        return Err(Error::Parse("require object missing closing brace".into()));
    };
    let before = &links_text[..close];
    let trail = &links_text[close..]; // `}`
    // Preserve whitespace before `}` as Composer does (`$match[1]`).
    let ws_start = before
        .rfind(|c: char| !c.is_whitespace())
        .map(|p| p + 1)
        .unwrap_or(0);
    let match1 = &before[ws_start..];
    let core = &before[..ws_start];
    Ok(format!(
        "{},{}{}{}{}: {}{}{}",
        core,
        newline,
        indent,
        indent,
        json_encode_string(package),
        json_encode_string(constraint),
        match1,
        trail
    ))
}

/// Read file text, add requirement, return new text (does not write).
pub fn add_requirement_preserving(
    contents: &str,
    name: &str,
    constraint: &str,
    dev: bool,
    sort_packages: bool,
) -> Result<String> {
    let mut manip = JsonManipulator::new(contents)?;
    let link_type = if dev { "require-dev" } else { "require" };
    manip.add_link(link_type, name, constraint, sort_packages)?;
    Ok(manip.get_contents())
}

/// Read file text, remove requirement from require/require-dev, return new text.
pub fn remove_requirement_preserving(contents: &str, name: &str) -> Result<(String, bool)> {
    let mut manip = JsonManipulator::new(contents)?;
    let removed = manip.remove_link(name)?;
    Ok((manip.get_contents(), removed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_sort_puts_php_first() {
        let mut map = Map::new();
        map.insert("webmozart/assert".into(), Value::String("^1".into()));
        map.insert("php".into(), Value::String("^8.3".into()));
        map.insert("laravel/framework".into(), Value::String("^13".into()));
        sort_packages_map(&mut map);
        let keys: Vec<_> = map.keys().cloned().collect();
        assert_eq!(keys, vec!["php", "laravel/framework", "webmozart/assert"]);
    }

    #[test]
    fn add_link_sorted_matches_composer_fixture() {
        let input = r#"{
    "name": "app/app",
    "require": {
        "php": "^8.3",
        "laravel/framework": "^13.0"
    },
    "config": {
        "sort-packages": true
    }
}"#;
        let out =
            add_requirement_preserving(input, "webmozart/assert", "^1.11", false, true).unwrap();
        let expected = r#"{
    "name": "app/app",
    "require": {
        "php": "^8.3",
        "laravel/framework": "^13.0",
        "webmozart/assert": "^1.11"
    },
    "config": {
        "sort-packages": true
    }
}
"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn add_link_unsorted_appends() {
        let input = r#"{
    "name": "app/app",
    "require": {
        "laravel/framework": "^13.0",
        "php": "^8.3"
    }
}"#;
        let out =
            add_requirement_preserving(input, "webmozart/assert", "^1.11", false, false).unwrap();
        let expected = r#"{
    "name": "app/app",
    "require": {
        "laravel/framework": "^13.0",
        "php": "^8.3",
        "webmozart/assert": "^1.11"
    }
}
"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn add_link_inserts_sorted_middle() {
        let input = r#"{
    "name": "app/app",
    "require": {
        "php": "^8.3",
        "webmozart/assert": "^1.11"
    },
    "config": {
        "sort-packages": true
    }
}"#;
        let out =
            add_requirement_preserving(input, "laravel/framework", "^13.0", false, true).unwrap();
        let expected = r#"{
    "name": "app/app",
    "require": {
        "php": "^8.3",
        "laravel/framework": "^13.0",
        "webmozart/assert": "^1.11"
    },
    "config": {
        "sort-packages": true
    }
}
"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn remove_link_preserves_rest() {
        let input = r#"{
    "name": "app/app",
    "require": {
        "php": "^8.3",
        "laravel/framework": "^13.0",
        "webmozart/assert": "^1.11"
    },
    "require-dev": {
        "phpunit/phpunit": "^11.0"
    },
    "config": {
        "sort-packages": true
    }
}"#;
        let (out, removed) = remove_requirement_preserving(input, "webmozart/assert").unwrap();
        assert!(removed);
        let expected = r#"{
    "name": "app/app",
    "require": {
        "php": "^8.3",
        "laravel/framework": "^13.0"
    },
    "require-dev": {
        "phpunit/phpunit": "^11.0"
    },
    "config": {
        "sort-packages": true
    }
}
"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn add_require_dev_main_key() {
        let input = r#"{
    "name": "app/app",
    "require": {
        "php": "^8.3"
    }
}"#;
        let out =
            add_requirement_preserving(input, "phpunit/phpunit", "^11.0", true, false).unwrap();
        let expected = r#"{
    "name": "app/app",
    "require": {
        "php": "^8.3"
    },
    "require-dev": {
        "phpunit/phpunit": "^11.0"
    }
}
"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn update_existing_constraint() {
        let input = r#"{
    "name": "app/app",
    "require": {
        "php": "^8.3",
        "laravel/framework": "^12.0"
    },
    "config": {
        "sort-packages": true
    }
}"#;
        let out =
            add_requirement_preserving(input, "laravel/framework", "^13.0", false, true).unwrap();
        let expected = r#"{
    "name": "app/app",
    "require": {
        "php": "^8.3",
        "laravel/framework": "^13.0"
    },
    "config": {
        "sort-packages": true
    }
}
"#;
        assert_eq!(out, expected);
    }
}
