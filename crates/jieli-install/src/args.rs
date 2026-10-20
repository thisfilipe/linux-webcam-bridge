use std::fmt;
use std::path::PathBuf;

pub const USAGE: &str = "usage: jieli-install [--plugin PATH] [--replace] [--install|--remove]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Plan,
    Install,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub mode: Mode,
    pub plugin: Option<PathBuf>,
    pub replace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgsError {
    Help,
    Message(String),
}

impl fmt::Display for ArgsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Help => write!(formatter, "{USAGE}"),
            Self::Message(message) => write!(formatter, "{message}"),
        }
    }
}

pub fn parse_args<I, S>(args: I) -> Result<Args, ArgsError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let values: Vec<String> = args
        .into_iter()
        .map(|value| value.as_ref().to_string())
        .collect();
    let mut mode = Mode::Plan;
    let mut mode_set = false;
    let mut plugin = None;
    let mut replace = false;
    let mut index = 0;

    while index < values.len() {
        let argument = values[index].as_str();
        match argument {
            "-h" | "--help" => return Err(ArgsError::Help),
            "--install" => {
                set_mode(&mut mode, &mut mode_set, Mode::Install)?;
                index += 1;
            }
            "--remove" => {
                set_mode(&mut mode, &mut mode_set, Mode::Remove)?;
                index += 1;
            }
            "--replace" => {
                replace = true;
                index += 1;
            }
            "--plugin" => {
                let value = values
                    .get(index + 1)
                    .ok_or_else(|| ArgsError::Message("--plugin requires a path".to_string()))?;
                if value.is_empty() || value.starts_with("--") {
                    return Err(ArgsError::Message("--plugin requires a path".to_string()));
                }
                plugin = Some(PathBuf::from(value));
                index += 2;
            }
            _ if argument.starts_with("--plugin=") => {
                let value = argument.trim_start_matches("--plugin=");
                if value.is_empty() {
                    return Err(ArgsError::Message("--plugin requires a path".to_string()));
                }
                plugin = Some(PathBuf::from(value));
                index += 1;
            }
            _ => {
                return Err(ArgsError::Message(format!(
                    "unrecognized argument: {argument}"
                )));
            }
        }
    }

    if mode == Mode::Remove && replace {
        return Err(ArgsError::Message(
            "--replace is only valid with --install or the default plan".to_string(),
        ));
    }

    Ok(Args {
        mode,
        plugin,
        replace,
    })
}

fn set_mode(current: &mut Mode, set: &mut bool, next: Mode) -> Result<(), ArgsError> {
    if *set {
        return Err(ArgsError::Message(
            "--install and --remove are mutually exclusive".to_string(),
        ));
    }
    *current = next;
    *set = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_a_plan() {
        assert_eq!(
            parse_args(std::iter::empty::<&str>()).unwrap(),
            Args {
                mode: Mode::Plan,
                plugin: None,
                replace: false
            }
        );
    }

    #[test]
    fn parses_plugin_forms_and_modes() {
        let separated =
            parse_args(["--plugin", "build/plugin/libgstpipewire.so", "--install"]).unwrap();
        assert_eq!(separated.mode, Mode::Install);
        assert_eq!(
            separated.plugin,
            Some(PathBuf::from("build/plugin/libgstpipewire.so"))
        );

        let attached = parse_args(["--plugin=build/p.so", "--remove", "--replace"]);
        assert_eq!(
            attached,
            Err(ArgsError::Message(
                "--replace is only valid with --install or the default plan".to_string()
            ))
        );
    }

    #[test]
    fn rejects_conflicting_and_incomplete_options() {
        assert!(matches!(
            parse_args(["--install", "--remove"]),
            Err(ArgsError::Message(_))
        ));
        assert!(matches!(
            parse_args(["--plugin"]),
            Err(ArgsError::Message(_))
        ));
        assert!(matches!(
            parse_args(["--plugin="]),
            Err(ArgsError::Message(_))
        ));
        assert!(matches!(
            parse_args(["--unknown"]),
            Err(ArgsError::Message(_))
        ));
    }

    #[test]
    fn help_is_not_an_error_message() {
        assert_eq!(parse_args(["--help"]), Err(ArgsError::Help));
    }
}
