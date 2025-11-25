use clap::Parser as ClapParser;
use miette::{IntoDiagnostic, NamedSource, Result};
use soppo::ast::Decl;
use soppo::codegen::Codegen;
use soppo::infer::Infer;
use soppo::parser::Parser;
use soppo::source::FileId;
use std::fs;
use std::path::PathBuf;

#[derive(ClapParser)]
#[command(name = "soppo")]
struct Cli {
    /// Input Soppo source file (.sop)
    input: PathBuf,

    /// Output Go file (defaults to input with .go extension)
    #[arg(short, long)]
    output: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Read source file
    let source = fs::read_to_string(&cli.input)
        .into_diagnostic()
        .map_err(|e| e.context(format!("Failed to read file: {}", cli.input.display())))?;

    let filename = cli
        .input
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("input.sop");

    // Compile with nice error reporting
    let output = compile(&source, filename)?;

    // Determine output path
    let output_path = cli.output.unwrap_or_else(|| {
        let mut path = cli.input.clone();
        path.set_extension("go");
        path
    });

    // Write output
    fs::write(&output_path, output)
        .into_diagnostic()
        .map_err(|e| e.context(format!("Failed to write file: {}", output_path.display())))?;

    println!("✓ Compiled {} → {}", filename, output_path.display());
    Ok(())
}

fn compile(source: &str, filename: &str) -> Result<String> {
    // Parse
    let mut parser = Parser::new(source, FileId(0));
    let file = parser.parse_file().map_err(|e| {
        miette::Report::from(e).with_source_code(NamedSource::new(filename, source.to_string()))
    })?;

    // Type check
    let mut infer = Infer::new();

    // Process imports to add package names to scope
    infer.process_imports(&file.imports);

    for decl in &file.decls {
        match decl {
            Decl::Const(const_decl) => {
                infer.infer_const_decl(const_decl).map_err(|e| {
                    miette::Report::from(e)
                        .with_source_code(NamedSource::new(filename, source.to_string()))
                })?;
            }
            Decl::Type(type_decl) => {
                infer.infer_type_decl(type_decl).map_err(|e| {
                    miette::Report::from(e)
                        .with_source_code(NamedSource::new(filename, source.to_string()))
                })?;
            }
            Decl::Func(func) => {
                infer.infer_func_decl(func).map_err(|e| {
                    miette::Report::from(e)
                        .with_source_code(NamedSource::new(filename, source.to_string()))
                })?;
            }
        }
    }

    // Generate Go code
    let global_state = infer.global_state();
    let mut codegen = Codegen::with_global_state(global_state);
    codegen.gen_file(&file);

    Ok(codegen.output().to_string())
}
