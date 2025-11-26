use std::fs;
use std::process::Command;

pub fn compile_soppo_file(input_file: &str) -> Result<String, String> {
    let output_file = input_file.replace(".sop", ".go");

    let output = Command::new("cargo")
        .env("RUSTFLAGS", "-A warnings")
        .args(&["run", "--quiet", "--bin", "sop", "--", "build", input_file])
        .output()
        .expect("Failed to run sop");

    if output.status.success() {
        let go_code = fs::read_to_string(&output_file).expect("Failed to read generated .go file");

        let vet_output = Command::new("go")
            .args(&["vet", &output_file])
            .output()
            .expect("Failed to run go vet");

        if !vet_output.status.success() {
            let vet_error = String::from_utf8_lossy(&vet_output.stderr);
            fs::remove_file(&output_file).ok();
            panic!(
                "Generated Go code failed go vet:\n{}\n\nGenerated code:\n{}",
                vet_error, go_code
            );
        }

        fs::remove_file(&output_file).ok();
        Ok(go_code)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}
