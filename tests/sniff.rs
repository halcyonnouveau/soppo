use std::fs;
use std::path::Path;
use std::sync::Once;

use soppo::build;
use soppo::sniff::{self, LintConfig};
use test_generator::test_resources;

static INIT: Once = Once::new();

fn init_no_color() {
    INIT.call_once(|| {
        // Disable colors in miette output
        // SAFETY: We're only setting this at test init, before any other threads
        unsafe {
            std::env::set_var("NO_COLOR", "1");
        }
    });
}

fn lint_soppo_file(path: &str) -> String {
    init_no_color();
    let source = fs::read_to_string(path).expect("Failed to read file");
    let filename = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("input.sop");

    let typed_file = match build::typecheck_to_typed(&source, filename) {
        Ok(f) => f,
        Err(e) => return format!("Compile error: {:?}", e),
    };

    let config = LintConfig::default();
    let warnings = sniff::lint_file(&typed_file, filename, &source, &config);

    if warnings.is_empty() {
        "No warnings".to_string()
    } else {
        warnings
            .into_iter()
            .map(|w| format!("{:?}", miette::Report::new(w)))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

#[test_resources("tests/fixtures/sniff/*.sop")]
fn test_sniff(resource: &str) {
    let output = lint_soppo_file(resource);

    let file_name = Path::new(resource)
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let name = format!("sniff__{}", file_name);

    insta::assert_snapshot!(name, output);
}
