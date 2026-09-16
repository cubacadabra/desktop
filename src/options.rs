use std::{env, error::Error, fmt, path::PathBuf};

#[derive(Debug)]
pub struct DesktopError(pub String);

impl fmt::Display for DesktopError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for DesktopError {}

pub struct Options {
    pub package: Option<PathBuf>,
}

pub fn parse() -> Result<Options, Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let mut package = None;
    while let Some(argument) = args.next() {
        if argument == "--help" || argument == "-h" {
            println!("Usage: desktop [--path <built-game-package>]");
            println!();
            println!("Run a built Cubacadabra game package.");
            std::process::exit(0);
        }
        if argument == "--path" {
            package = Some(
                args.next()
                    .ok_or_else(|| DesktopError("--path expects a package directory".into()))?,
            );
        } else if package.is_none() {
            package = Some(argument);
        } else {
            return Err(Box::new(DesktopError(format!(
                "unknown argument: {}",
                argument.to_string_lossy()
            ))));
        }
    }
    let package = package
        .map(PathBuf::from)
        .map(|package| {
            package.canonicalize().and_then(|package| {
                if package.is_dir() {
                    Ok(package)
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("package is not a directory: {}", package.display()),
                    ))
                }
            })
        })
        .transpose()?;
    Ok(Options { package })
}
