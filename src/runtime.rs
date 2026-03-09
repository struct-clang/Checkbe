use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::module_system::ModuleRegistry;

#[derive(Clone, Debug)]
pub struct RuntimeArtifacts {
    pub core_object: PathBuf,
    pub module_libraries: Vec<PathBuf>,
    pub gc_lib_dir: PathBuf,
}

#[derive(Clone, Debug)]
struct GcConfig {
    include_dir: PathBuf,
    lib_dir: PathBuf,
}

pub fn discover_runtime_roots(source_path: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(root) = env::var("CHECKBE_RUNTIME") {
        roots.push(PathBuf::from(root));
    }

    if let Some(source_dir) = source_path.parent() {
        roots.push(source_dir.join("runtime"));
    }

    if let Ok(current_dir) = env::current_dir() {
        roots.push(current_dir.join("runtime"));
    }

    if let Ok(home) = env::var("HOME") {
        roots.push(PathBuf::from(&home).join(".local/share/checkbe/runtime"));
        roots.push(PathBuf::from(home).join(".checkbe/runtime"));
    }

    roots.push(PathBuf::from("/usr/local/lib/checkbe/runtime"));
    roots.push(PathBuf::from("/opt/homebrew/lib/checkbe/runtime"));

    if let Ok(exe_path) = env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            roots.push(exe_dir.join("runtime"));
            if let Some(parent) = exe_dir.parent() {
                roots.push(parent.join("runtime"));
            }
        }
    }

    dedup_paths(roots)
}

pub fn pick_runtime_root(search_roots: &[PathBuf]) -> Option<PathBuf> {
    for root in search_roots {
        if root.join("core").join("runtime.c").exists() {
            return Some(root.clone());
        }
    }
    None
}

pub fn ensure_runtime(
    runtime_root: &Path,
    modules: &ModuleRegistry,
) -> Result<RuntimeArtifacts, String> {
    let gc = discover_gc_config()?;

    let cache_root = runtime_build_root(runtime_root);
    let lib_dir = cache_root.join("lib");
    let build_dir = cache_root.join("build");
    fs::create_dir_all(&lib_dir)
        .map_err(|error| format!("Failed to create {}: {}", lib_dir.display(), error))?;
    fs::create_dir_all(&build_dir)
        .map_err(|error| format!("Failed to create {}: {}", build_dir.display(), error))?;

    let core_source = runtime_root.join("core").join("runtime.c");
    if !core_source.exists() {
        return Err(format!(
            "Runtime core source not found: {}",
            core_source.display()
        ));
    }

    let core_object = build_dir.join("runtime_core.o");
    compile_c_file(&core_source, &core_object, Some(&gc.include_dir))?;

    let mut module_libraries = Vec::new();
    let mut compiled_libraries = HashSet::new();

    let mut module_names: Vec<&String> = modules.modules().map(|(name, _)| name).collect();
    module_names.sort();

    for module_name in module_names {
        let Some(module) = modules.module(module_name) else {
            continue;
        };

        let library_name = module.spec.library.clone();
        if !compiled_libraries.insert(library_name.clone()) {
            continue;
        }

        let module_build_dir = build_dir.join(module_name);
        fs::create_dir_all(&module_build_dir).map_err(|error| {
            format!(
                "Failed to create build directory for module {}: {}",
                module_name, error
            )
        })?;

        let mut objects = Vec::new();
        for source in &module.spec.native_sources {
            let source_path = module.root_dir.join(source);
            if !source_path.exists() {
                return Err(format!(
                    "Native source '{}' not found for module '{}' ({})",
                    source,
                    module_name,
                    source_path.display()
                ));
            }
            let object_path = module_build_dir.join(source_to_object_name(source));
            compile_c_file(&source_path, &object_path, Some(&gc.include_dir))?;
            objects.push(object_path);
        }

        if objects.is_empty() {
            continue;
        }

        let library_path = lib_dir.join(format!("lib{}.a", library_name));
        archive_static_library(&library_path, &objects)?;
        module_libraries.push(library_path);
    }

    Ok(RuntimeArtifacts {
        core_object,
        module_libraries,
        gc_lib_dir: gc.lib_dir,
    })
}

pub fn link_binary(
    object_path: &Path,
    output_path: &Path,
    artifacts: &RuntimeArtifacts,
) -> Result<(), String> {
    let mut args: Vec<String> = vec![
        object_path.display().to_string(),
        artifacts.core_object.display().to_string(),
    ];

    for library in &artifacts.module_libraries {
        args.push(library.display().to_string());
    }

    args.push(format!("-L{}", artifacts.gc_lib_dir.display()));
    args.push("-lgc".to_string());
    args.push("-lm".to_string());
    args.push(format!("-Wl,-rpath,{}", artifacts.gc_lib_dir.display()));
    args.push("-o".to_string());
    args.push(output_path.display().to_string());

    run_command("clang", &args)
}

fn compile_c_file(source: &Path, output: &Path, include_dir: Option<&Path>) -> Result<(), String> {
    let mut args = vec![
        "-c".to_string(),
        source.display().to_string(),
        "-o".to_string(),
        output.display().to_string(),
        "-std=c11".to_string(),
    ];

    if let Some(include_dir) = include_dir {
        args.push(format!("-I{}", include_dir.display()));
    }

    run_command("clang", &args)
}

fn archive_static_library(library_path: &Path, objects: &[PathBuf]) -> Result<(), String> {
    let mut args = vec!["rcs".to_string(), library_path.display().to_string()];
    for object in objects {
        args.push(object.display().to_string());
    }

    run_command("ar", &args)
}

fn run_command(command: &str, args: &[String]) -> Result<(), String> {
    let output = Command::new(command)
        .args(args)
        .output()
        .map_err(|error| format!("Failed to start {}: {}", command, error))?;

    if output.status.success() {
        return Ok(());
    }

    Err(format!(
        "Command '{}' failed with code {:?}\nstdout:\n{}\nstderr:\n{}",
        command,
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

fn source_to_object_name(source: &str) -> String {
    let path = Path::new(source);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("module");
    format!("{}.o", stem)
}

fn discover_gc_config() -> Result<GcConfig, String> {
    if let (Ok(include_dir), Ok(lib_dir)) =
        (env::var("CHECKBE_GC_INCLUDE"), env::var("CHECKBE_GC_LIB"))
    {
        return Ok(GcConfig {
            include_dir: PathBuf::from(include_dir),
            lib_dir: PathBuf::from(lib_dir),
        });
    }

    if let Some(config) = discover_gc_with_pkg_config() {
        return Ok(config);
    }

    let include = PathBuf::from("/opt/homebrew/opt/bdw-gc/include");
    let lib = PathBuf::from("/opt/homebrew/opt/bdw-gc/lib");
    if include.exists() && lib.exists() {
        return Ok(GcConfig {
            include_dir: include,
            lib_dir: lib,
        });
    }

    Err(
        "Could not detect Boehm GC. Install bdw-gc or set CHECKBE_GC_INCLUDE/CHECKBE_GC_LIB"
            .to_string(),
    )
}

fn discover_gc_with_pkg_config() -> Option<GcConfig> {
    let output = Command::new("pkg-config")
        .args(["--cflags", "--libs", "bdw-gc"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8(output.stdout).ok()?;
    let mut include_dir = None;
    let mut lib_dir = None;

    for part in text.split_whitespace() {
        if let Some(value) = part.strip_prefix("-I") {
            include_dir = Some(PathBuf::from(value));
        }
        if let Some(value) = part.strip_prefix("-L") {
            lib_dir = Some(PathBuf::from(value));
        }
    }

    Some(GcConfig {
        include_dir: include_dir?,
        lib_dir: lib_dir?,
    })
}

fn dedup_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();

    for path in paths {
        if seen.insert(path.clone()) {
            unique.push(path);
        }
    }

    unique
}

fn runtime_build_root(runtime_root: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    runtime_root.to_string_lossy().hash(&mut hasher);
    let key = hasher.finish();
    env::temp_dir()
        .join("checkbe_runtime_build")
        .join(format!("{key:016x}"))
}
