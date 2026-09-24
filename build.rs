use cubacadabra_builder::{BuildOptions, build_game};
use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/logs/HEAD");
    let git_sha = Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .current_dir(&manifest_dir)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|sha| sha.trim().to_owned())
        .filter(|sha| sha.len() == 8 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .unwrap_or_else(|| "UNKNOWN".to_owned());
    println!("cargo:rustc-env=CUBACADABRA_GIT_SHA={git_sha}");
    let workspace_root = manifest_dir
        .parent()
        .ok_or("desktop repository has no workspace parent")?;
    let game_roots = [
        ("first-game", workspace_root.join("first-game")),
        ("second-game", workspace_root.join("second-game")),
    ];
    let tools_root = workspace_root.join("tools");
    let output_root = PathBuf::from(env::var("OUT_DIR")?).join("game-packages");

    for (_, game_root) in &game_roots {
        println!("cargo:rerun-if-changed={}", game_root.display());
    }
    println!("cargo:rerun-if-changed={}", tools_root.display());
    if !tools_root.join("Cargo.toml").is_file()
        || !tools_root.join("crates/cli/Cargo.toml").is_file()
    {
        return Err(format!(
            "the shared Cubacadabra tools checkout is missing: {}",
            tools_root.display()
        )
        .into());
    }

    let mut packages = String::new();
    for (game_id, game_root) in &game_roots {
        if !game_root.join("manifest.json").is_file() || !game_root.join("src/main.luau").is_file()
        {
            return Err(format!(
                "the bundled {game_id} project is missing manifest.json or src/main.luau: {}",
                game_root.display()
            )
            .into());
        }

        let package_output = output_root.join(game_id);
        let build_options = BuildOptions {
            source_root: game_root.join("src"),
            manifest_path: game_root.join("manifest.json"),
            output: package_output.clone(),
            zip_path: None,
        };
        // Windows gives build scripts a smaller default stack than Unix hosts.
        // The builder walks the Luau module graph recursively, so run it on an
        // explicitly sized stack to keep cross-platform builds equivalent.
        std::thread::Builder::new()
            .name(format!("cubacadabra-{game_id}-builder"))
            .stack_size(8 * 1024 * 1024)
            .spawn(move || build_game(&build_options))
            .map_err(|error| {
                std::io::Error::other(format!("could not start game builder: {error}"))
            })?
            .join()
            .map_err(|_| std::io::Error::other("bundled game builder thread panicked"))??;

        let mut files = Vec::new();
        collect_files(&package_output, &package_output, &mut files)?;
        files.sort();
        if !files.iter().any(|path| path == "manifest.json")
            || !files.iter().any(|path| path == "game.luau")
        {
            return Err(format!(
                "the bundled {game_id} package did not contain manifest.json and game.luau"
            )
            .into());
        }

        let generated_files = files
            .iter()
            .map(|path| {
                let include_path =
                    format!("concat!(env!(\"OUT_DIR\"), \"/game-packages/{game_id}/{path}\")");
                format!(
                    "        ({path:?}, include_bytes!({include_path})),\n",
                    path = path,
                    include_path = include_path
                )
            })
            .collect::<String>();
        packages.push_str(&format!(
            "    BundledPackage {{\n        id: {game_id:?},\n        manifest: include_str!(concat!(env!(\"OUT_DIR\"), \"/game-packages/{game_id}/manifest.json\")),\n        script: include_str!(concat!(env!(\"OUT_DIR\"), \"/game-packages/{game_id}/game.luau\")),\n        files: &[\n{generated_files}        ],\n    }},\n"
        ));
    }
    let source = format!(
        "pub struct BundledPackage {{\n    pub id: &'static str,\n    pub manifest: &'static str,\n    pub script: &'static str,\n    pub files: &'static [(&'static str, &'static [u8])],\n}}\n\
         pub static PACKAGES: &[BundledPackage] = &[\n{packages}];\n"
    );
    fs::write(
        PathBuf::from(env::var("OUT_DIR")?).join("bundled_game.rs"),
        source,
    )?;
    Ok(())
}

fn collect_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<String>,
) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, files)?;
        } else if path.is_file() {
            files.push(
                path.strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    Ok(())
}
