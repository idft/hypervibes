use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Unit tests render templates directly and do not need compiled frontend assets.
    if std::env::var_os("CARGO_CFG_TEST").is_some()
        && std::env::var_os("HYPERVIBES_FORCE_FRONTEND_BUILD").is_none()
    {
        return Ok(());
    }

    println!("cargo:rerun-if-changed=assets/");
    println!("cargo:rerun-if-changed=package.json");
    println!("cargo:rerun-if-changed=pnpm-lock.yaml");
    println!("cargo:rerun-if-changed=pnpm-workspace.yaml");
    println!("cargo:rerun-if-changed=templates/");

    let root = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR")
            .expect("Cargo must set CARGO_MANIFEST_DIR for build scripts"),
    );
    let dist = root.join("static/dist");
    let js_out = dist.join("app.js");
    let css_out = dist.join("app.css");

    let mut needs_rebuild = !js_out.exists() || !css_out.exists();

    if !needs_rebuild {
        let js_modified = js_out.metadata()?.modified()?;
        let css_modified = css_out.metadata()?.modified()?;
        let out_modified = js_modified.min(css_modified);

        let deps = [
            root.join("package.json"),
            root.join("pnpm-lock.yaml"),
            root.join("pnpm-workspace.yaml"),
            root.join("assets/app.css"),
        ];
        for dep in &deps {
            if dep.metadata()?.modified()? > out_modified {
                needs_rebuild = true;
                break;
            }
        }

        if !needs_rebuild && let Ok(true) = any_newer_than(&root.join("assets"), "ts", out_modified)
        {
            needs_rebuild = true;
        }

        if !needs_rebuild {
            // Tailwind output depends on the classes used in templates.
            if let Ok(true) = any_newer_than(&root.join("templates"), "html", out_modified) {
                needs_rebuild = true;
            }
        }
    }

    if !needs_rebuild {
        return Ok(());
    }

    fs::create_dir_all(&dist)?;

    let pnpm_check = Command::new("pnpm").arg("--version").output();
    match pnpm_check {
        Ok(output) if output.status.success() => {}
        _ => {
            return Err(
                "pnpm is required to build frontend assets. Install it from https://pnpm.io/installation"
                    .into(),
            );
        }
    }

    let node_modules = root.join("node_modules");
    let lockfile = root.join("pnpm-lock.yaml");
    let should_install = !node_modules.exists()
        || lockfile.metadata()?.modified()? > node_modules.metadata()?.modified()?;

    if should_install {
        let status = Command::new("pnpm")
            .args(["install", "--frozen-lockfile"])
            .current_dir(&root)
            .status()?;
        if !status.success() {
            return Err("pnpm install failed".into());
        }
    }

    let status = Command::new("pnpm")
        .arg("build")
        .current_dir(&root)
        .status()?;
    if !status.success() {
        return Err("pnpm build failed".into());
    }

    Ok(())
}

fn any_newer_than(dir: &Path, extension: &str, than: SystemTime) -> io::Result<bool> {
    visit_dirs(dir, extension, than)
}

fn visit_dirs(dir: &Path, extension: &str, than: SystemTime) -> io::Result<bool> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if visit_dirs(&path, extension, than)? {
                return Ok(true);
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some(extension)
            && path.metadata()?.modified()? > than
        {
            return Ok(true);
        }
    }
    Ok(false)
}
