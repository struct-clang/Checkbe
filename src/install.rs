use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub struct InstallResult {
    pub binary_path: PathBuf,
    pub runtime_root: PathBuf,
}

pub fn install(prefix: &Path) -> Result<InstallResult, String> {
    let exe =
        env::current_exe().map_err(|err| format!("Failed to locate current executable: {err}"))?;

    let bin_dir = prefix.join("bin");
    fs::create_dir_all(&bin_dir)
        .map_err(|err| format!("Failed to create {}: {err}", bin_dir.display()))?;

    let binary_path = bin_dir.join("checkbe");
    fs::copy(&exe, &binary_path).map_err(|err| {
        format!(
            "Failed to copy binary {} -> {}: {}",
            exe.display(),
            binary_path.display(),
            err
        )
    })?;

    #[cfg(unix)]
    {
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(&binary_path, perms).map_err(|err| {
            format!(
                "Failed to set executable permissions on {}: {}",
                binary_path.display(),
                err
            )
        })?;
    }

    let runtime_root = prefix.join("lib/checkbe/runtime");
    install_runtime_sources(&runtime_root)?;

    Ok(InstallResult {
        binary_path,
        runtime_root,
    })
}

fn install_runtime_sources(runtime_root: &Path) -> Result<(), String> {
    let core_dir = runtime_root.join("core");
    let bridge_dir = runtime_root.join("modules/Bridge");

    fs::create_dir_all(&core_dir)
        .map_err(|err| format!("Failed to create {}: {err}", core_dir.display()))?;
    fs::create_dir_all(&bridge_dir)
        .map_err(|err| format!("Failed to create {}: {err}", bridge_dir.display()))?;

    write_file(
        &core_dir.join("runtime.c"),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/runtime/core/runtime.c"
        )),
    )?;
    write_file(
        &bridge_dir.join("module.toml"),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/runtime/modules/Bridge/module.toml"
        )),
    )?;
    write_file(
        &bridge_dir.join("bridge.c"),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/runtime/modules/Bridge/bridge.c"
        )),
    )?;

    Ok(())
}

fn write_file(path: &Path, content: &str) -> Result<(), String> {
    fs::write(path, content).map_err(|err| format!("Failed to write {}: {err}", path.display()))
}
