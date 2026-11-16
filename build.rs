use std::{env, fs, path::{Path, PathBuf}, process::Command};

fn npm(project: &Path, args: &[&str]) {
    let mut command = if cfg!(windows) {
        let mut command = Command::new("cmd.exe");
        command.args(["/d", "/c", "npm"]);
        command
    } else {
        Command::new("npm")
    };
    let status = command.current_dir(project).args(args).status()
        .expect("Install Node.js 22 or newer and npm to build the MediaBridge dashboard");
    assert!(status.success(), "Dashboard command npm {} failed", args.join(" "));
}

fn collect_assets(root: &Path, directory: &Path, assets: &mut Vec<(String, PathBuf)>) {
    for entry in fs::read_dir(directory).expect("read dashboard build output") {
        let entry = entry.expect("read dashboard asset");
        let path = entry.path();
        if entry.file_type().expect("read dashboard asset type").is_dir() {
            collect_assets(root, &path, assets);
        } else {
            let name = path.strip_prefix(root).unwrap().to_str().unwrap().replace('\\', "/");
            assets.push((format!("/{name}"), path));
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var_os("CARGO_FEATURE_NATIVE_DASHBOARD").is_none() { return; }
    let project = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("dashboard-preview/svelte");
    for source in ["src", "static", "package.json", "package-lock.json", "svelte.config.js", "vite.config.ts", "tsconfig.json"] {
        println!("cargo:rerun-if-changed={}", project.join(source).display());
    }
    npm(&project, &["ci", "--no-audit", "--no-fund"]);
    npm(&project, &["run", "build"]);

    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let root = project.join("build");
    assert!(root.join("index.html").is_file(), "Missing dashboard SPA entry point");
    let mut assets = Vec::new();
    collect_assets(&root, &root, &mut assets);
    assets.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let mut table = String::from("static ASSETS: &[(&str, &[u8])] = &[\n");
    for (url, path) in assets {
        let asset_path = output.join(url.trim_start_matches('/'));
        fs::create_dir_all(asset_path.parent().unwrap()).expect("create embedded asset directory");
        fs::copy(path, &asset_path).expect("copy embedded dashboard asset");
        table.push_str(&format!("    ({url:?}, include_bytes!({asset_path:?})),\n"));
    }
    table.push_str("];\n");
    fs::write(output.join("dashboard_assets.rs"), table).expect("write embedded dashboard manifest");
}
