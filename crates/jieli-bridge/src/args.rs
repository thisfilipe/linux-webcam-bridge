use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeArgs {
    pub test_source: bool,
    pub node_name: String,
    pub state_file: PathBuf,
}

pub const USAGE: &str =
    "usage: jieli-bridge [--test-source] [--node-name NAME] [--state-file PATH]";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgsError {
    Help,
    MissingValue(String),
    Unknown(String),
}

impl fmt::Display for ArgsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Help => write!(formatter, "{USAGE}"),
            Self::MissingValue(option) => {
                write!(formatter, "argument {option}: expected one argument")
            }
            Self::Unknown(argument) => write!(formatter, "unrecognized arguments: {argument}"),
        }
    }
}

pub fn default_state_file() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("jieli-camera-bridge.json")
}

pub fn parse_args<I, S>(args: I) -> Result<BridgeArgs, ArgsError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|value| value.as_ref().to_string())
        .collect();
    let mut test_source = false;
    let mut node_name = "jieli_720p30".to_string();
    let mut state_file = default_state_file();
    let mut index = 0;

    while index < args.len() {
        let argument = args[index].as_str();
        match argument {
            "-h" | "--help" => return Err(ArgsError::Help),
            "--test-source" => {
                test_source = true;
                index += 1;
            }
            "--node-name" => {
                node_name = take_value(&args, index, "--node-name")?;
                index += 2;
            }
            "--state-file" => {
                state_file = PathBuf::from(take_value(&args, index, "--state-file")?);
                index += 2;
            }
            _ if argument.starts_with("--node-name=") => {
                node_name = non_empty(argument.trim_start_matches("--node-name="), "--node-name")?;
                index += 1;
            }
            _ if argument.starts_with("--state-file=") => {
                state_file = PathBuf::from(non_empty(
                    argument.trim_start_matches("--state-file="),
                    "--state-file",
                )?);
                index += 1;
            }
            _ => return Err(ArgsError::Unknown(argument.to_string())),
        }
    }

    Ok(BridgeArgs {
        test_source,
        node_name,
        state_file,
    })
}

fn take_value(args: &[String], index: usize, option: &str) -> Result<String, ArgsError> {
    let value = args
        .get(index + 1)
        .ok_or_else(|| ArgsError::MissingValue(option.to_string()))?;
    if value.starts_with("--") {
        return Err(ArgsError::MissingValue(option.to_string()));
    }
    Ok(value.clone())
}

fn non_empty(value: &str, option: &str) -> Result<String, ArgsError> {
    if value.is_empty() {
        Err(ArgsError::MissingValue(option.to_string()))
    } else {
        Ok(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_defaults_and_test_source() {
        let defaults = parse_args(Vec::<String>::new()).unwrap();
        assert!(!defaults.test_source);
        assert_eq!(defaults.node_name, "jieli_720p30");
        assert!(defaults.state_file.ends_with("jieli-camera-bridge.json"));

        let test = parse_args([
            "--test-source",
            "--node-name",
            "jieli_validation",
            "--state-file",
            "/tmp/test-state.json",
        ])
        .unwrap();
        assert!(test.test_source);
        assert_eq!(test.node_name, "jieli_validation");
        assert_eq!(test.state_file, PathBuf::from("/tmp/test-state.json"));
    }

    #[test]
    fn supports_attached_values() {
        let args = parse_args(["--node-name=jieli_test", "--state-file=/tmp/state.json"]).unwrap();
        assert_eq!(args.node_name, "jieli_test");
        assert_eq!(args.state_file, PathBuf::from("/tmp/state.json"));
    }

    #[test]
    fn rejects_missing_values_and_unknown_arguments() {
        assert_eq!(
            parse_args(["--node-name"]),
            Err(ArgsError::MissingValue("--node-name".to_string()))
        );
        assert_eq!(
            parse_args(["--state-file", "--test-source"]),
            Err(ArgsError::MissingValue("--state-file".to_string()))
        );
        assert_eq!(
            parse_args(["--unknown"]),
            Err(ArgsError::Unknown("--unknown".to_string()))
        );
        assert_eq!(parse_args(["--help"]), Err(ArgsError::Help));
    }
}
