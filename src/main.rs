mod ast;
mod codegen;
mod diagnostics;
mod install;
mod lexer;
mod module_system;
mod multi_source;
mod parser;
mod runtime;
mod sema;
mod span;
mod string_interp;
mod token;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use diagnostics::{Diagnostics, Severity};
use multi_source::ParsedSource;

fn main() {
    if let Err(error) = run() {
        eprintln!("{} {}", style_tag(Severity::Error), error);
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    let command = parse_cli_args(&args)?;

    let CliCommand::Compile {
        source_paths,
        output_path,
    } = command
    else {
        let CliCommand::Install { prefix } = command else {
            unreachable!();
        };
        let result = install::install(&prefix)?;
        println!(
            "{} Installed compiler: {}",
            success_tag(),
            result.binary_path.display()
        );
        println!(
            "{} Installed runtime: {}",
            success_tag(),
            result.runtime_root.display()
        );
        return Ok(());
    };

    let mut parsed_sources = Vec::with_capacity(source_paths.len());
    for source_path in &source_paths {
        if source_path
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("checkbe")
        {
            return Err(format!(
                "Expected an input file with .checkbe extension, got: {}",
                source_path.display()
            ));
        }

        let source = fs::read_to_string(source_path)
            .map_err(|error| format!("Failed to read {}: {}", source_path.display(), error))?;

        let mut diagnostics = Diagnostics::default();
        let tokens = lexer::lex(&source, &mut diagnostics);
        if diagnostics.has_errors() {
            print_diagnostics(source_path, &diagnostics);
            return Err("Lexical analysis failed".to_string());
        }

        let Some(program) = parser::parse(tokens, &mut diagnostics) else {
            print_diagnostics(source_path, &diagnostics);
            return Err("Parsing failed".to_string());
        };

        print_diagnostics(source_path, &diagnostics);
        parsed_sources.push(ParsedSource {
            path: source_path.clone(),
            program,
        });
    }

    let mut diagnostics = Diagnostics::default();
    let Some(program) = multi_source::merge_sources(&parsed_sources, &mut diagnostics) else {
        print_diagnostics(&source_paths[0], &diagnostics);
        return Err("Source merge failed".to_string());
    };
    print_diagnostics(&source_paths[0], &diagnostics);

    let runtime_roots = runtime::discover_runtime_roots(&source_paths[0]);
    let module_registry = module_system::ModuleRegistry::load_for_imports(
        &program.imports,
        &runtime_roots,
        &mut diagnostics,
    );

    let Some(semantic_model) = sema::analyze(&program, module_registry, &mut diagnostics) else {
        print_diagnostics(&source_paths[0], &diagnostics);
        return Err("Semantic analysis failed".to_string());
    };

    print_diagnostics(&source_paths[0], &diagnostics);

    let runtime_root = runtime::pick_runtime_root(&runtime_roots).ok_or_else(|| {
        format!(
            "Runtime root not found. Expected runtime/core/runtime.c in: {}",
            runtime_roots
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;

    let temp_dir =
        tempfile::tempdir().map_err(|error| format!("Failed to create temp dir: {}", error))?;
    let object_path = temp_dir.path().join("program.o");

    codegen::generate_object(&program, &semantic_model, &object_path)?;
    let artifacts = runtime::ensure_runtime(&runtime_root, &semantic_model.modules)?;
    runtime::link_binary(&object_path, &output_path, &artifacts)?;

    println!(
        "{} Compiled {} file(s) -> {}",
        success_tag(),
        source_paths.len(),
        output_path.display()
    );
    Ok(())
}

enum CliCommand {
    Compile {
        source_paths: Vec<PathBuf>,
        output_path: PathBuf,
    },
    Install {
        prefix: PathBuf,
    },
}

fn parse_cli_args(args: &[String]) -> Result<CliCommand, String> {
    if args.len() < 2 {
        return Err(usage());
    }

    if args[1] == "--help" || args[1] == "-h" {
        return Err(usage());
    }

    if args[1] == "--install" {
        let mut prefix = PathBuf::from("/usr/local");
        let mut index = 2;
        while index < args.len() {
            match args[index].as_str() {
                "--prefix" => {
                    let value = args.get(index + 1).ok_or_else(usage)?;
                    prefix = PathBuf::from(value);
                    index += 2;
                }
                unknown => return Err(format!("Unknown argument '{}'.\n{}", unknown, usage())),
            }
        }
        return Ok(CliCommand::Install { prefix });
    }

    let mut source_paths = Vec::new();
    let mut output = None;

    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "-o" => {
                let value = args.get(index + 1).ok_or_else(usage)?;
                output = Some(PathBuf::from(value));
                index += 2;
            }
            value if value.starts_with('-') => {
                return Err(format!("Unknown argument '{}'.\n{}", value, usage()));
            }
            source => {
                source_paths.push(PathBuf::from(source));
                index += 1;
            }
        }
    }

    if source_paths.is_empty() {
        return Err(usage());
    }

    let output_path = output.unwrap_or_else(|| default_output_path(&source_paths[0]));
    Ok(CliCommand::Compile {
        source_paths,
        output_path,
    })
}

fn default_output_path(source_path: &Path) -> PathBuf {
    source_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("a.out"))
}

fn usage() -> String {
    "Usage:\n  checkbe <source.checkbe> [more.checkbe ...] -o <output_binary>\n  checkbe --install [--prefix <path>]".to_string()
}

fn print_diagnostics(source_path: &Path, diagnostics: &Diagnostics) {
    for diagnostic in diagnostics.items() {
        let tag = style_tag(diagnostic.severity);
        match diagnostic.span {
            Some(span) => eprintln!(
                "{} {}:{}:{}: {}",
                tag,
                source_path.display(),
                span.line,
                span.column,
                diagnostic.message
            ),
            None => eprintln!("{} {}", tag, diagnostic.message),
        }
    }

    if diagnostics.warnings_count() > 0 || diagnostics.errors_count() > 0 {
        let summary_tag = if diagnostics.errors_count() > 0 {
            style_tag(Severity::Error)
        } else {
            style_tag(Severity::Warning)
        };
        eprintln!(
            "{} Diagnostics: {} warning(s), {} error(s)",
            summary_tag,
            diagnostics.warnings_count(),
            diagnostics.errors_count()
        );
    }
}

fn style_tag(severity: Severity) -> &'static str {
    match severity {
        Severity::Warning => "\x1b[43;97;1m[WARN]\x1b[0m",
        Severity::Error => "\x1b[41;97;1m[ERROR]\x1b[0m",
    }
}

fn success_tag() -> &'static str {
    "\x1b[42;97;1m[SUCCESS]\x1b[0m"
}
