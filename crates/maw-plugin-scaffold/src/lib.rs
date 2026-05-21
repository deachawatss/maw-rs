//! Pure plugin scaffold helpers ported from maw-js
//! `src/commands/shared/plugin-create-scaffold.ts`.
//!
//! This crate ports the deterministic validation/manifest helpers plus the
//! template tree-copy, Rust/AssemblyScript scaffold, and command guard
//! contracts from `test/plugin-create.test.ts`.

use std::{fs, io, path::Path};

use serde_json::{json, Map, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginCreateRequest {
    pub name: Option<String>,
    pub rust: bool,
    pub assembly_script: bool,
    pub dest: std::path::PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginCreateError {
    MissingType,
    ConflictingTypes,
    MissingName,
    InvalidName(String),
    DestinationExists(std::path::PathBuf),
    Scaffold(String),
}

impl std::fmt::Display for PluginCreateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingType => write!(
                f,
                "usage: maw plugin create [--rust | --as] <name> [--here]\n  Specify either --rust or --as"
            ),
            Self::ConflictingTypes => write!(f, "  Specify --rust or --as, not both"),
            Self::MissingName => write!(f, "usage: maw plugin create [--rust | --as] <name> [--here]"),
            Self::InvalidName(error) => write!(f, "✗ Invalid plugin name: {error}"),
            Self::DestinationExists(dest) => write!(f, "✗ Destination already exists: {}", dest.display()),
            Self::Scaffold(error) => write!(f, "✗ {error}"),
        }
    }
}

impl std::error::Error for PluginCreateError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginLanguage {
    Rust,
    AssemblyScript,
}

/// Execute the pure command guard and dispatch for `maw plugin create`.
///
/// This mirrors the command-boundary checks in maw-js `cmdPluginCreate`, but
/// returns typed errors instead of calling `process.exit(1)`.
///
/// # Errors
///
/// Returns validation, destination-exists, or scaffold filesystem errors.
pub fn cmd_plugin_create(
    request: &PluginCreateRequest,
    rust_template_dir: impl AsRef<Path>,
    as_template_dir: impl AsRef<Path>,
    sdk_path: &str,
) -> Result<(), PluginCreateError> {
    if !request.rust && !request.assembly_script {
        return Err(PluginCreateError::MissingType);
    }
    if request.rust && request.assembly_script {
        return Err(PluginCreateError::ConflictingTypes);
    }
    let Some(name) = request.name.as_deref() else {
        return Err(PluginCreateError::MissingName);
    };
    if let Some(error) = validate_plugin_name(name) {
        return Err(PluginCreateError::InvalidName(error));
    }
    if request.dest.exists() {
        return Err(PluginCreateError::DestinationExists(request.dest.clone()));
    }

    let result = if request.rust {
        scaffold_rust(name, &request.dest, rust_template_dir, sdk_path)
    } else {
        scaffold_as(name, &request.dest, as_template_dir)
    };
    result.map_err(|error| PluginCreateError::Scaffold(error.to_string()))
}

/// Copy a scaffold template tree while skipping build and package artifacts.
///
/// Mirrors maw-js `copyTree`: create the destination directory, recurse into
/// subdirectories, copy files, and skip `target`, `.git`, and `node_modules`
/// entries wherever they appear.
///
/// # Errors
///
/// Returns filesystem errors from reading the source tree, creating
/// directories, or copying files.
pub fn copy_tree(src: impl AsRef<Path>, dest: impl AsRef<Path>) -> io::Result<()> {
    copy_tree_inner(src.as_ref(), dest.as_ref())
}

/// Scaffold a Rust WASM plugin from a template directory.
///
/// Mirrors maw-js `scaffoldRust`: validates the template exists, copies the
/// template tree, rewrites `Cargo.toml` package name and SDK path, writes a
/// README, and emits `plugin.json`.
///
/// # Errors
///
/// Returns filesystem errors from template lookup, tree copy, reading/writing
/// `Cargo.toml`, README, or `plugin.json`.
pub fn scaffold_rust(
    name: &str,
    dest: impl AsRef<Path>,
    template_dir: impl AsRef<Path>,
    sdk_path: &str,
) -> io::Result<()> {
    let dest = dest.as_ref();
    let template_dir = template_dir.as_ref();
    if !template_dir.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Rust template not found at {}", template_dir.display()),
        ));
    }

    copy_tree(template_dir, dest)?;

    let cargo_path = dest.join("Cargo.toml");
    let cargo = fs::read_to_string(&cargo_path)?;
    let cargo = rewrite_rust_cargo_toml(&cargo, name, sdk_path);
    fs::write(&cargo_path, cargo)?;

    fs::write(dest.join("README.md"), rust_readme(name, dest, sdk_path))?;
    fs::write(
        dest.join("plugin.json"),
        build_manifest_json(name, PluginLanguage::Rust),
    )?;
    Ok(())
}

/// Scaffold an `AssemblyScript` WASM plugin from a template directory.
///
/// Mirrors maw-js `scaffoldAs`: validates the template exists, copies the
/// template tree, rewrites package.json name when present, writes a README,
/// and emits `plugin.json`.
///
/// # Errors
///
/// Returns filesystem errors from template lookup, tree copy, reading/writing
/// `package.json`, README, or `plugin.json`, plus invalid package JSON.
pub fn scaffold_as(
    name: &str,
    dest: impl AsRef<Path>,
    template_dir: impl AsRef<Path>,
) -> io::Result<()> {
    let dest = dest.as_ref();
    let template_dir = template_dir.as_ref();
    if !template_dir.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "AssemblyScript template not found at {}\n  The AS SDK is still being built — try again after the next maw update,\n  or check: https://github.com/Soul-Brews-Studio/maw-js",
                template_dir.display()
            ),
        ));
    }

    copy_tree(template_dir, dest)?;

    let package_path = dest.join("package.json");
    if package_path.exists() {
        let package = fs::read_to_string(&package_path)?;
        let package = rewrite_package_json_name(&package, name)?;
        fs::write(&package_path, package)?;
    }

    fs::write(dest.join("README.md"), as_readme(name, dest))?;
    fs::write(
        dest.join("plugin.json"),
        build_manifest_json(name, PluginLanguage::AssemblyScript),
    )?;
    Ok(())
}

/// Validate a plugin scaffold name.
///
/// Returns `None` for valid names and the maw-js error text for invalid names.
#[must_use]
pub fn validate_plugin_name(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("name is required".to_owned());
    }
    if !is_valid_plugin_name(name) {
        return Some(format!(
            "\"{name}\" is invalid — use lowercase letters, digits, - or _ (must start with a letter)"
        ));
    }
    None
}

/// Build plugin.json content for a scaffolded plugin.
///
/// Underscores are normalized to hyphens for slug fields, while Rust wasm crate
/// artifacts normalize hyphens to underscores like maw-js.
///
/// # Panics
///
/// Panics only if `serde_json` cannot serialize the statically constructed manifest.
#[must_use]
pub fn build_manifest_json(name: &str, lang: PluginLanguage) -> String {
    let slug = name.replace('_', "-");
    let wasm_path = match lang {
        PluginLanguage::Rust => format!(
            "./target/wasm32-unknown-unknown/release/{}.wasm",
            name.replace('-', "_")
        ),
        PluginLanguage::AssemblyScript => "./build/release.wasm".to_owned(),
    };
    let type_name = match lang {
        PluginLanguage::Rust => "Rust",
        PluginLanguage::AssemblyScript => "AssemblyScript",
    };

    let mut manifest = Map::new();
    manifest.insert("name".to_owned(), json!(slug));
    manifest.insert("version".to_owned(), json!("0.1.0"));
    manifest.insert("wasm".to_owned(), json!(wasm_path));
    manifest.insert("sdk".to_owned(), json!("^1.0.0"));
    manifest.insert(
        "description".to_owned(),
        json!(format!("{type_name} plugin: {name}")),
    );
    manifest.insert("author".to_owned(), json!(""));
    manifest.insert(
        "cli".to_owned(),
        json!({ "command": slug, "help": format!("Invoke {name}") }),
    );
    manifest.insert(
        "api".to_owned(),
        json!({ "path": format!("/api/plugins/{slug}"), "methods": ["GET", "POST"] }),
    );

    let text = serde_json::to_string_pretty(&Value::Object(manifest))
        .expect("plugin manifest JSON serialization should be infallible");
    format!("{text}\n")
}

fn rewrite_package_json_name(package: &str, name: &str) -> io::Result<String> {
    let mut value: Value = serde_json::from_str(package).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("package.json: invalid JSON: {error}"),
        )
    })?;
    match &mut value {
        Value::Object(object) => {
            object.insert("name".to_owned(), Value::String(name.to_owned()));
            let text = serde_json::to_string_pretty(&value)
                .expect("package.json serialization should be infallible");
            Ok(format!("{text}\n"))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "package.json: must be a JSON object",
        )),
    }
}

fn as_readme(name: &str, dest: &Path) -> String {
    format!(
        r#"# {name}

A maw WASM command plugin (AssemblyScript).

## Build

```bash
cd "{}"
npm install
npm run build
```

Output: `build/{name}.wasm`

## Install

```bash
maw plugin install "{}"
```
"#,
        dest.display(),
        dest.display()
    )
}

fn rewrite_rust_cargo_toml(cargo: &str, name: &str, sdk_path: &str) -> String {
    let mut rewritten = cargo
        .lines()
        .map(|line| {
            if line.starts_with("name = ") {
                format!(r#"name = "{name}""#)
            } else if line.trim_start().starts_with("maw-plugin-sdk = { path = ") {
                format!(r#"maw-plugin-sdk = {{ path = "{sdk_path}" }}"#)
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if cargo.ends_with('\n') {
        rewritten.push('\n');
    }
    rewritten
}

fn rust_readme(name: &str, dest: &Path, sdk_path: &str) -> String {
    let crate_name = name.replace('-', "_");
    format!(
        r#"# {name}

A maw WASM command plugin (Rust).

## Build

```bash
cd "{}"
cargo build --release --target wasm32-unknown-unknown
```

Output: `target/wasm32-unknown-unknown/release/{crate_name}.wasm`

## Install

```bash
maw plugin install "{}"
```

## SDK docs

See the SDK at `{sdk_path}` for available host functions:
`maw::print`, `maw::identity`, `maw::federation`, `maw::send`, `maw::fetch`.
"#,
        dest.display(),
        dest.display()
    )
}

fn copy_tree_inner(src: &Path, dest: &Path) -> io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_name = entry.file_name();
        if should_skip_entry(&file_name) {
            continue;
        }
        let source_path = entry.path();
        let dest_path = dest.join(file_name);
        if entry.file_type()?.is_dir() {
            copy_tree_inner(&source_path, &dest_path)?;
        } else {
            fs::copy(&source_path, &dest_path)?;
        }
    }
    Ok(())
}

fn should_skip_entry(name: &std::ffi::OsStr) -> bool {
    matches!(name.to_str(), Some("target" | ".git" | "node_modules"))
}

fn is_valid_plugin_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("maw-plugin-scaffold-{name}-{nonce}"))
    }

    #[test]
    fn scaffold_rust_writes_manifest_readme_and_rewritten_cargo() {
        let template = temp_dir("rust-template");
        let dest = temp_dir("rust-dest");
        fs::create_dir_all(&template).expect("create template");
        fs::write(
            template.join("Cargo.toml"),
            "name = \"template\"\nmaw-plugin-sdk = { path = \"../old\" }\n",
        )
        .expect("write cargo");

        scaffold_rust("hello-plugin", &dest, &template, "../sdk").expect("scaffold rust");

        let cargo = fs::read_to_string(dest.join("Cargo.toml")).expect("read cargo");
        assert!(cargo.contains("name = \"hello-plugin\""));
        assert!(cargo.contains("maw-plugin-sdk = { path = \"../sdk\" }"));
        let manifest = fs::read_to_string(dest.join("plugin.json")).expect("read manifest");
        assert!(manifest.contains("\"name\": \"hello-plugin\""));
        assert!(fs::read_to_string(dest.join("README.md"))
            .expect("read readme")
            .contains("hello-plugin"));
        let _ = fs::remove_dir_all(template);
        let _ = fs::remove_dir_all(dest);
    }

    #[test]
    fn scaffold_as_rewrites_package_and_writes_manifest() {
        let template = temp_dir("as-template");
        let dest = temp_dir("as-dest");
        fs::create_dir_all(&template).expect("create template");
        fs::write(template.join("package.json"), r#"{"name":"template"}"#).expect("write package");

        scaffold_as("hello_as", &dest, &template).expect("scaffold as");

        assert!(fs::read_to_string(dest.join("package.json"))
            .expect("read package")
            .contains("\"name\": \"hello_as\""));
        assert!(fs::read_to_string(dest.join("plugin.json"))
            .expect("read manifest")
            .contains("\"name\": \"hello-as\""));
        let _ = fs::remove_dir_all(template);
        let _ = fs::remove_dir_all(dest);
    }

    #[test]
    fn scaffold_edges_cover_package_without_manifest_and_name_start() {
        let template = temp_dir("as-template-no-package");
        let dest = temp_dir("as-dest-no-package");
        fs::create_dir_all(&template).expect("create template");

        scaffold_as("edge_plugin", &dest, &template).expect("scaffold as without package");

        assert!(dest.join("plugin.json").exists());
        assert!(!dest.join("package.json").exists());
        assert_eq!(
            validate_plugin_name("1bad"),
            Some(
                "\"1bad\" is invalid — use lowercase letters, digits, - or _ (must start with a letter)"
                    .to_owned()
            )
        );
        let _ = fs::remove_dir_all(template);
        let _ = fs::remove_dir_all(dest);
    }

    #[test]
    fn invalid_empty_plugin_name_is_rejected() {
        assert_eq!(
            validate_plugin_name("").as_deref(),
            Some("name is required")
        );
        assert!(validate_plugin_name("1bad").is_some());
        assert!(!is_valid_plugin_name(""));
    }
}
