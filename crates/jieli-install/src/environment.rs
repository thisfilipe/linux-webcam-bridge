use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    pub home: PathBuf,
    pub config: PathBuf,
    pub state: PathBuf,
    pub root: PathBuf,
}

impl Environment {
    pub fn current() -> Result<Self, String> {
        let home = required_absolute("HOME")?;
        let config = xdg_directory("XDG_CONFIG_HOME")?.unwrap_or_else(|| home.join(".config"));
        let state = xdg_directory("XDG_STATE_HOME")?.unwrap_or_else(|| home.join(".local/state"));
        let root = env::current_dir()
            .map_err(|error| format!("cannot determine the current directory: {error}"))?;
        Ok(Self {
            home,
            config,
            state,
            root,
        })
    }
}

fn required_absolute(name: &str) -> Result<PathBuf, String> {
    let value = env::var_os(name)
        .ok_or_else(|| format!("{name} is not set; run from your desktop user session"))?;
    let path = PathBuf::from(value);
    if path.as_os_str().is_empty() {
        return Err(format!("{name} is empty"));
    }
    if !path.is_absolute() {
        return Err(format!("{name} must be an absolute path"));
    }
    Ok(path)
}

fn xdg_directory(name: &str) -> Result<Option<PathBuf>, String> {
    let Some(value) = env::var_os(name) else {
        return Ok(None);
    };
    let path = PathBuf::from(value);
    if path.as_os_str().is_empty() {
        return Ok(None);
    }
    if !path.is_absolute() {
        return Err(format!("{name} must be an absolute path"));
    }
    Ok(Some(path))
}
