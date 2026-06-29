use maw_plugin_scaffold::{copy_tree, scaffold_as, scaffold_rust, validate_plugin_name};
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("maw-plugin-scaffold-{name}-{nonce}"))
}

fn running_as_root() -> bool {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .is_some_and(|uid| uid.trim() == "0")
}

#[test]
fn scaffold_rust_writes_manifest_and_readme() {
    let root = temp_dir("rust");
    let template = root.join("template");
    let dest = root.join("dest");
    fs::create_dir_all(&template).expect("create template");
    fs::write(
        template.join("Cargo.toml"),
        r#"[package]
name = "template"
[dependencies]
maw-plugin-sdk = { path = "old" }
"#,
    )
    .expect("write cargo template");

    scaffold_rust("hello-world", &dest, &template, "../sdk").expect("scaffold rust");

    assert!(fs::read_to_string(dest.join("plugin.json"))
        .expect("read manifest")
        .contains("hello-world"));
    assert!(fs::read_to_string(dest.join("README.md"))
        .expect("read readme")
        .contains("hello-world"));
    assert!(fs::read_to_string(dest.join("Cargo.toml"))
        .expect("read cargo")
        .contains(r#"maw-plugin-sdk = { path = "../sdk" }"#));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn scaffold_as_writes_manifest_and_rewrites_package() {
    let root = temp_dir("as");
    let template = root.join("template");
    let dest = root.join("dest");
    fs::create_dir_all(&template).expect("create template");
    fs::write(template.join("package.json"), r#"{"name":"template"}"#)
        .expect("write package template");

    scaffold_as("hello_as", &dest, &template).expect("scaffold as");

    assert!(fs::read_to_string(dest.join("plugin.json"))
        .expect("read manifest")
        .contains("hello-as"));
    assert!(fs::read_to_string(dest.join("README.md"))
        .expect("read readme")
        .contains("hello_as"));
    assert!(fs::read_to_string(dest.join("package.json"))
        .expect("read package")
        .contains("hello_as"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn empty_plugin_name_fails_validator() {
    assert_eq!(
        validate_plugin_name(""),
        Some("name is required".to_owned())
    );
}

#[test]
fn scaffold_rust_reports_plugin_json_write_error() {
    let root = temp_dir("rust-plugin-json-dir");
    let template = root.join("template");
    let dest = root.join("dest");
    fs::create_dir_all(template.join("plugin.json")).expect("create plugin dir");
    fs::write(
        template.join("Cargo.toml"),
        r#"[package]
name = "template"
[dependencies]
maw-plugin-sdk = { path = "old" }
"#,
    )
    .expect("write cargo template");

    let error = scaffold_rust("hello-world", &dest, &template, "../sdk")
        .expect_err("plugin.json directory should reject manifest write");
    assert!(
        error.to_string().contains("Is a directory")
            || error.kind() == std::io::ErrorKind::PermissionDenied
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn scaffold_as_reports_plugin_json_write_error() {
    let root = temp_dir("as-plugin-json-dir");
    let template = root.join("template");
    let dest = root.join("dest");
    fs::create_dir_all(template.join("plugin.json")).expect("create plugin dir");

    let error = scaffold_as("hello-as", &dest, &template)
        .expect_err("plugin.json directory should reject manifest write");
    assert!(
        error.to_string().contains("Is a directory")
            || error.kind() == std::io::ErrorKind::PermissionDenied
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn copy_tree_reports_create_dir_and_read_dir_errors() {
    let root = temp_dir("copy-errors");
    fs::create_dir_all(&root).expect("create root");
    let src_file = root.join("src-file");
    let dest_file = root.join("dest-file");
    fs::write(&src_file, "not a dir").expect("write src file");
    fs::write(&dest_file, "not a dir").expect("write dest file");

    let create_error = copy_tree(&src_file, dest_file.join("child"))
        .expect_err("create_dir_all below file should fail");
    assert!(matches!(
        create_error.kind(),
        std::io::ErrorKind::NotADirectory | std::io::ErrorKind::AlreadyExists
    ));

    let read_error = copy_tree(&src_file, root.join("dest-dir"))
        .expect_err("reading a file as a directory should fail");
    assert!(matches!(
        read_error.kind(),
        std::io::ErrorKind::NotADirectory | std::io::ErrorKind::InvalidInput
    ));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn scaffold_reports_cargo_package_read_and_package_write_errors() {
    let root = temp_dir("midstream-errors");
    let rust_template = root.join("rust-template");
    let rust_dest = root.join("rust-dest");
    fs::create_dir_all(rust_template.join("Cargo.toml")).expect("cargo dir");
    let rust_error = scaffold_rust("hello-world", &rust_dest, &rust_template, "../sdk")
        .expect_err("Cargo.toml directory should reject read");
    assert!(
        rust_error.to_string().contains("Is a directory")
            || rust_error.kind() == std::io::ErrorKind::PermissionDenied
    );

    let as_template = root.join("as-template");
    let as_dest = root.join("as-dest");
    fs::create_dir_all(&as_template).expect("as template");
    fs::write(as_template.join("package.json"), r#"{"name":"template"}"#)
        .expect("package template");
    fs::create_dir_all(as_dest.join("package.json")).expect("dest package dir");
    let as_error = scaffold_as("hello-as", &as_dest, &as_template)
        .expect_err("package.json directory should reject rewrite");
    assert!(
        as_error.to_string().contains("Is a directory")
            || as_error.kind() == std::io::ErrorKind::PermissionDenied
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn scaffold_rust_reports_copy_and_write_failures() {
    let root = temp_dir("rust-copy-write-failures");
    let template = root.join("template");
    fs::create_dir_all(&template).expect("create template");
    fs::write(
        template.join("Cargo.toml"),
        "name = \"template\"\nmaw-plugin-sdk = { path = \"old\" }\n",
    )
    .expect("write cargo template");

    let blocking_file = root.join("blocking-file");
    fs::write(&blocking_file, "not a directory").expect("write blocking file");
    let copy_error = scaffold_rust(
        "copy-fail",
        blocking_file.join("plugin"),
        &template,
        "../sdk",
    )
    .expect_err("copy_tree failure should propagate from scaffold_rust");
    assert!(matches!(
        copy_error.kind(),
        std::io::ErrorKind::NotADirectory | std::io::ErrorKind::AlreadyExists
    ));

    if running_as_root() {
        eprintln!(
            "skip readonly Cargo.toml rewrite assertion: root bypasses OS readonly permissions"
        );
    } else {
        let readonly_dest = root.join("readonly-dest");
        let cargo_path = template.join("Cargo.toml");
        let original_permissions = fs::metadata(&cargo_path)
            .expect("cargo metadata")
            .permissions();
        let mut readonly_permissions = original_permissions.clone();
        readonly_permissions.set_readonly(true);
        fs::set_permissions(&cargo_path, readonly_permissions).expect("make cargo readonly");
        let write_error = scaffold_rust("write-fail", &readonly_dest, &template, "../sdk")
            .expect_err("readonly Cargo.toml should reject rewrite");
        assert!(matches!(
            write_error.kind(),
            std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
        ));
        let _ = fs::set_permissions(
            readonly_dest.join("Cargo.toml"),
            original_permissions.clone(),
        );
        let _ = fs::set_permissions(&cargo_path, original_permissions);
    }

    let readme_template = root.join("readme-template");
    let readme_dest = root.join("readme-dest");
    fs::create_dir_all(readme_template.join("README.md")).expect("readme dir");
    fs::write(
        readme_template.join("Cargo.toml"),
        "name = \"template\"\nmaw-plugin-sdk = { path = \"old\" }\n",
    )
    .expect("write cargo template");
    let readme_error = scaffold_rust("readme-fail", &readme_dest, &readme_template, "../sdk")
        .expect_err("README.md directory should reject readme write");
    assert!(
        readme_error.to_string().contains("Is a directory")
            || readme_error.kind() == std::io::ErrorKind::PermissionDenied
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn scaffold_as_reports_package_and_readme_write_failures() {
    let root = temp_dir("as-write-failures");
    let template = root.join("template");
    fs::create_dir_all(&template).expect("create template");
    fs::write(template.join("package.json"), r#"{"name":"template"}"#)
        .expect("write package template");

    if running_as_root() {
        eprintln!(
            "skip readonly package.json rewrite assertion: root bypasses OS readonly permissions"
        );
    } else {
        let package_path = template.join("package.json");
        let original_permissions = fs::metadata(&package_path)
            .expect("package metadata")
            .permissions();
        let mut readonly_permissions = original_permissions.clone();
        readonly_permissions.set_readonly(true);
        fs::set_permissions(&package_path, readonly_permissions).expect("make package readonly");
        let package_dest = root.join("package-dest");
        let package_error = scaffold_as("package-fail", &package_dest, &template)
            .expect_err("readonly package.json should reject rewrite");
        assert!(matches!(
            package_error.kind(),
            std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
        ));
        let _ = fs::set_permissions(
            package_dest.join("package.json"),
            original_permissions.clone(),
        );
        let _ = fs::set_permissions(&package_path, original_permissions);
    }

    let readme_template = root.join("readme-template");
    let readme_dest = root.join("readme-dest");
    fs::create_dir_all(readme_template.join("README.md")).expect("readme dir");
    let readme_error = scaffold_as("readme-fail", &readme_dest, &readme_template)
        .expect_err("README.md directory should reject AS readme write");
    assert!(
        readme_error.to_string().contains("Is a directory")
            || readme_error.kind() == std::io::ErrorKind::PermissionDenied
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn copy_tree_reports_recursive_destination_conflict() {
    let root = temp_dir("copy-recursive-conflict");
    let src = root.join("src");
    let dest = root.join("dest");
    fs::create_dir_all(src.join("nested")).expect("nested source");
    fs::create_dir_all(&dest).expect("dest root");
    fs::write(dest.join("nested"), "not a directory").expect("blocking dest file");

    let error = copy_tree(&src, &dest).expect_err("nested dest file should reject recursive copy");
    assert!(matches!(
        error.kind(),
        std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::NotADirectory
    ));
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn copy_tree_reports_unreadable_source_file() {
    use std::os::unix::fs::PermissionsExt;

    let root = temp_dir("copy-tree-unreadable-file");
    let src = root.join("src");
    let dest = root.join("dest");
    let file = src.join("secret.txt");
    fs::create_dir_all(&src).expect("src");
    fs::write(&file, "secret").expect("source file");
    let original = fs::metadata(&file).expect("metadata").permissions();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).expect("chmod unreadable");

    let error = copy_tree(&src, &dest).expect_err("unreadable source should reject copy");

    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    let _ = fs::set_permissions(&file, original);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn scaffold_as_reports_missing_template_path() {
    let root = temp_dir("as-missing-template");
    let error = scaffold_as(
        "missing-as",
        root.join("dest"),
        root.join("missing-template"),
    )
    .expect_err("missing AssemblyScript template should fail");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert!(error
        .to_string()
        .contains("AssemblyScript template not found"));
}
