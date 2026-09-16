use cubacadabra_builder::{build_game, BuildOptions};
use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
};

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let workspace_root = manifest_dir
        .parent()
        .ok_or("desktop repository has no workspace parent")?;
    let game_root = workspace_root.join("first-game");
    let tools_root = workspace_root.join("tools");
    let output_root = PathBuf::from(env::var("OUT_DIR")?).join("first-game-package");

    for path in [&game_root, &tools_root] {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    if !game_root.join("manifest.json").is_file() || !game_root.join("src/main.luau").is_file() {
        return Err(format!(
            "the bundled first-game project is missing: {}",
            game_root.display()
        )
        .into());
    }
    if !tools_root.join("Cargo.toml").is_file()
        || !tools_root.join("crates/cli/Cargo.toml").is_file()
    {
        return Err(format!(
            "the shared Cubacadabra tools checkout is missing: {}",
            tools_root.display()
        )
        .into());
    }

    let build_options = BuildOptions {
        source_root: game_root.join("src"),
        manifest_path: game_root.join("manifest.json"),
        output: output_root.clone(),
        zip_path: None,
    };
    // Windows gives build scripts a smaller default stack than Unix hosts.
    // The builder walks the Luau module graph recursively, so run it on an
    // explicitly sized stack to keep cross-platform builds equivalent.
    std::thread::Builder::new()
        .name("cubacadabra-game-builder".to_owned())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || build_game(&build_options))
        .map_err(|error| std::io::Error::other(format!("could not start game builder: {error}")))?
        .join()
        .map_err(|_| std::io::Error::other("bundled game builder thread panicked"))??;

    let mut files = Vec::new();
    collect_files(&output_root, &output_root, &mut files)?;
    files.sort();
    if !files.iter().any(|path| path == "manifest.json")
        || !files.iter().any(|path| path == "game.luau")
    {
        return Err(
            "the bundled first-game package did not contain manifest.json and game.luau".into(),
        );
    }

    let generated = files
        .iter()
        .map(|path| {
            let include_path = format!(
                "concat!(env!(\"OUT_DIR\"), \"/first-game-package/{}\")",
                path
            );
            format!(
                "    ({path:?}, include_bytes!({include_path})),\n",
                path = path,
                include_path = include_path
            )
        })
        .collect::<String>();
    let source = format!(
        "pub const GAME_ID: &str = \"first-game\";\n\
         pub const MANIFEST: &str = include_str!(concat!(env!(\"OUT_DIR\"), \"/first-game-package/manifest.json\"));\n\
         pub const SCRIPT: &str = include_str!(concat!(env!(\"OUT_DIR\"), \"/first-game-package/game.luau\"));\n\
         pub static FILES: &[(&str, &[u8])] = &[\n{generated}         ];\n"
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
