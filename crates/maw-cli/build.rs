use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
struct PartFile {
    number: u32,
    path: PathBuf,
}

fn main() {
    if let Err(error) = generate() {
        panic!("failed to generate maw-cli core includes: {error}");
    }
}

fn generate() -> io::Result<()> {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set"));
    let core_impl_dir = manifest_dir.join("src").join("core_impl");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR set"));

    println!("cargo:rerun-if-changed=src/core_impl");

    let parts = collect_part_files(&core_impl_dir)?;
    let mut includes = String::new();
    let mut dispatch_numbers = Vec::new();

    for part in &parts {
        println!("cargo:rerun-if-changed={}", part.path.display());
        writeln!(includes, "include!({:?});", part.path.display().to_string())
            .expect("write to String");

        let contents = fs::read_to_string(&part.path)?;
        if let Some(dispatch_number) = find_dispatch_const_number(&contents) {
            assert_eq!(
                dispatch_number,
                part.number,
                "{} declares DISPATCH_{dispatch_number:02}, expected DISPATCH_{:02}",
                part.path.display(),
                part.number
            );
            dispatch_numbers.push(dispatch_number);
        }
    }

    let mut fragments =
        String::from("#[allow(clippy::needless_borrow)]\npub(crate) const DISPATCHER_FRAGMENTS: &[&[DispatcherEntry]] = &[\n");
    for number in dispatch_numbers {
        writeln!(fragments, "    &DISPATCH_{number:02},").expect("write to String");
    }
    fragments.push_str("];\n");

    fs::write(out_dir.join("parts_includes.rs"), includes)?;
    fs::write(out_dir.join("dispatch_fragments.rs"), fragments)?;
    Ok(())
}

fn collect_part_files(core_impl_dir: &Path) -> io::Result<Vec<PartFile>> {
    let mut parts = Vec::new();
    for entry in fs::read_dir(core_impl_dir)? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(number) = part_number(file_name) else {
            continue;
        };
        parts.push(PartFile { number, path });
    }

    parts.sort_by_key(|part| part.number);
    for window in parts.windows(2) {
        assert_ne!(
            window[0].number, window[1].number,
            "duplicate core_impl part number {:02}",
            window[0].number
        );
    }
    Ok(parts)
}

fn part_number(file_name: &str) -> Option<u32> {
    let rest = file_name.strip_prefix("part")?.strip_suffix(".rs")?;
    if rest.is_empty() || !rest.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

fn find_dispatch_const_number(contents: &str) -> Option<u32> {
    contents.lines().find_map(dispatch_const_number_from_line)
}

fn dispatch_const_number_from_line(line: &str) -> Option<u32> {
    let line = line.trim_start();
    let rest = line
        .strip_prefix("const ")
        .or_else(|| line.strip_prefix("pub const "))
        .or_else(|| line.strip_prefix("pub(crate) const "))?;
    let rest = rest.strip_prefix("DISPATCH_")?;
    let digits_len = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits_len == 0 || !rest[digits_len..].starts_with(':') {
        return None;
    }
    rest[..digits_len].parse().ok()
}
