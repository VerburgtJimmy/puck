//! Manifest types.

use crate::{Error, Result};
use indexmap::IndexMap;
use serde_json::{Map, Value};

/// Parsed and normalised root `composer.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    /// Lowercased package name (`Composer\Package\BasePackage`).
    pub name: String,
    /// Original `name` as written in the file.
    pub pretty_name: String,
    /// Lowercased type; defaults to `library`.
    pub package_type: String,
    pub description: Option<String>,
    pub license: Option<Value>,
    pub require: PackageLinks,
    pub require_dev: PackageLinks,
    pub conflict: PackageLinks,
    pub replace: PackageLinks,
    pub provide: PackageLinks,
    pub autoload: Option<Autoload>,
    pub autoload_dev: Option<Autoload>,
    pub scripts: IndexMap<String, Value>,
    pub config: Map<String, Value>,
    pub extra: Map<String, Value>,
    pub repositories: Value,
    pub minimum_stability: Option<String>,
    pub prefer_stable: Option<bool>,
    /// Remaining top-level keys (`$schema`, `keywords`, …).
    pub rest: Map<String, Value>,
}

/// Map of package/link name -> constraint string (order preserved).
pub type PackageLinks = IndexMap<String, String>;

/// Autoload section (PSR-4 / PSR-0 / classmap / files).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Autoload {
    pub psr4: IndexMap<String, Value>,
    pub psr0: IndexMap<String, Value>,
    pub classmap: Vec<Value>,
    pub files: Vec<Value>,
    pub exclude_from_classmap: Vec<Value>,
    pub rest: Map<String, Value>,
}

impl Manifest {
    pub fn from_value(value: Value) -> Result<Self> {
        let mut obj = match value {
            Value::Object(map) => map,
            _ => return Err(Error::RootNotObject),
        };

        let pretty_name = match obj.remove("name") {
            Some(Value::String(s)) if !s.is_empty() => s,
            Some(_) => return Err(Error::Parse("composer.json name must be a string".into())),
            None => return Err(Error::MissingName),
        };
        let name = pretty_name.to_ascii_lowercase();

        let package_type = match obj.remove("type") {
            Some(Value::String(s)) => s.to_ascii_lowercase(),
            Some(_) => return Err(Error::Parse("composer.json type must be a string".into())),
            None => "library".to_owned(),
        };

        let description = match obj.remove("description") {
            Some(Value::String(s)) => Some(s),
            Some(_) => {
                return Err(Error::Parse(
                    "composer.json description must be a string".into(),
                ));
            }
            None => None,
        };

        let license = obj.remove("license");

        let require = take_links(&mut obj, "require")?;
        let require_dev = take_links(&mut obj, "require-dev")?;
        let conflict = take_links(&mut obj, "conflict")?;
        let replace = take_links(&mut obj, "replace")?;
        let provide = take_links(&mut obj, "provide")?;

        let autoload = take_autoload(&mut obj, "autoload")?;
        let autoload_dev = take_autoload(&mut obj, "autoload-dev")?;

        let scripts = take_string_keyed_object(&mut obj, "scripts")?;
        let config = take_object(&mut obj, "config")?;
        let extra = take_object(&mut obj, "extra")?;
        let repositories = obj.remove("repositories").unwrap_or(Value::Null);

        let minimum_stability = match obj.remove("minimum-stability") {
            Some(Value::String(s)) => Some(s),
            Some(_) => return Err(Error::Parse("minimum-stability must be a string".into())),
            None => None,
        };
        let prefer_stable = match obj.remove("prefer-stable") {
            Some(Value::Bool(b)) => Some(b),
            Some(_) => return Err(Error::Parse("prefer-stable must be a boolean".into())),
            None => None,
        };

        Ok(Self {
            name,
            pretty_name,
            package_type,
            description,
            license,
            require,
            require_dev,
            conflict,
            replace,
            provide,
            autoload,
            autoload_dev,
            scripts,
            config,
            extra,
            repositories,
            minimum_stability,
            prefer_stable,
            rest: obj,
        })
    }

    /// Whether `allow-plugins` permits a given plugin package name.
    ///
    /// Mirrors Composer: missing key means deny in Composer 2.2+ for new
    /// projects when not listed; we treat explicit `true` / package / `*` .
    pub fn allows_plugin(&self, package: &str) -> bool {
        let Some(allow) = self.config.get("allow-plugins") else {
            return false;
        };
        match allow {
            Value::Bool(true) => true,
            Value::Bool(false) => false,
            Value::Object(map) => {
                if map.get("*").and_then(Value::as_bool) == Some(true) {
                    return true;
                }
                map.get(package)
                    .or_else(|| map.get(&package.to_ascii_lowercase()))
                    .and_then(Value::as_bool)
                    == Some(true)
            }
            _ => false,
        }
    }

    /// `config.optimize-autoloader` (Composer). Absent means false.
    pub fn optimize_autoloader(&self) -> bool {
        self.config
            .get("optimize-autoloader")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
}

fn take_links(obj: &mut Map<String, Value>, key: &str) -> Result<PackageLinks> {
    let Some(value) = obj.remove(key) else {
        return Ok(IndexMap::new());
    };
    let Value::Object(map) = value else {
        return Err(Error::Parse(format!("{key} must be an object")));
    };
    let mut links = IndexMap::new();
    for (target, constraint) in map {
        let Value::String(constraint) = constraint else {
            return Err(Error::Parse(format!(
                "{key}.{target} constraint must be a string"
            )));
        };
        // Platform packages (php, ext-*) keep their case as written for the
        // pretty form, but Composer lowercases link targets uniformly.
        links.insert(target.to_ascii_lowercase(), constraint);
    }
    Ok(links)
}

fn take_object(obj: &mut Map<String, Value>, key: &str) -> Result<Map<String, Value>> {
    match obj.remove(key) {
        None => Ok(Map::new()),
        Some(Value::Object(map)) => Ok(map),
        Some(_) => Err(Error::Parse(format!("{key} must be an object"))),
    }
}

fn take_string_keyed_object(
    obj: &mut Map<String, Value>,
    key: &str,
) -> Result<IndexMap<String, Value>> {
    match obj.remove(key) {
        None => Ok(IndexMap::new()),
        Some(Value::Object(map)) => Ok(map.into_iter().collect()),
        Some(_) => Err(Error::Parse(format!("{key} must be an object"))),
    }
}

fn take_autoload(obj: &mut Map<String, Value>, key: &str) -> Result<Option<Autoload>> {
    let Some(value) = obj.remove(key) else {
        return Ok(None);
    };
    let Value::Object(mut map) = value else {
        return Err(Error::Parse(format!("{key} must be an object")));
    };

    let psr4 = take_index_map(&mut map, "psr-4")?;
    let psr0 = take_index_map(&mut map, "psr-0")?;
    let classmap = take_array(&mut map, "classmap")?;
    let files = take_array(&mut map, "files")?;
    let exclude_from_classmap = take_array(&mut map, "exclude-from-classmap")?;

    Ok(Some(Autoload {
        psr4,
        psr0,
        classmap,
        files,
        exclude_from_classmap,
        rest: map,
    }))
}

fn take_index_map(obj: &mut Map<String, Value>, key: &str) -> Result<IndexMap<String, Value>> {
    match obj.remove(key) {
        None => Ok(IndexMap::new()),
        Some(Value::Object(map)) => Ok(map.into_iter().collect()),
        Some(_) => Err(Error::Parse(format!("{key} must be an object"))),
    }
}

fn take_array(obj: &mut Map<String, Value>, key: &str) -> Result<Vec<Value>> {
    match obj.remove(key) {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => Ok(items),
        Some(_) => Err(Error::Parse(format!("{key} must be an array"))),
    }
}
