use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublisherArgs {
    pub node_name: String,
    pub description: String,
}

pub const USAGE: &str = "usage: jieli-publisher --node-name NAME --description DESCRIPTION";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgsError {
    Help,
    Missing(&'static str),
    MissingValue(String),
    Unknown(String),
}

impl fmt::Display for ArgsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Help => write!(formatter, "{USAGE}"),
            Self::Missing(option) => {
                write!(formatter, "the following argument is required: {option}")
            }
            Self::MissingValue(option) => {
                write!(formatter, "argument {option}: expected one argument")
            }
            Self::Unknown(argument) => write!(formatter, "unrecognized arguments: {argument}"),
        }
    }
}

/// Parse the live publisher interface: both options are required, values may be
/// separated or attached with `=`, and other arguments are rejected.
pub fn parse_args<I, S>(args: I) -> Result<PublisherArgs, ArgsError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|value| value.as_ref().to_string())
        .collect();
    let mut node_name = None;
    let mut description = None;
    let mut index = 0;

    while index < args.len() {
        let argument = args[index].as_str();
        match argument {
            "-h" | "--help" => return Err(ArgsError::Help),
            "--node-name" => {
                node_name = Some(take_value(&args, index, "--node-name")?);
                index += 2;
            }
            "--description" => {
                description = Some(take_value(&args, index, "--description")?);
                index += 2;
            }
            _ if argument.starts_with("--node-name=") => {
                let value = argument.trim_start_matches("--node-name=");
                if value.is_empty() {
                    return Err(ArgsError::MissingValue("--node-name".to_string()));
                }
                node_name = Some(value.to_string());
                index += 1;
            }
            _ if argument.starts_with("--description=") => {
                let value = argument.trim_start_matches("--description=");
                if value.is_empty() {
                    return Err(ArgsError::MissingValue("--description".to_string()));
                }
                description = Some(value.to_string());
                index += 1;
            }
            _ => return Err(ArgsError::Unknown(argument.to_string())),
        }
    }

    Ok(PublisherArgs {
        node_name: node_name.ok_or(ArgsError::Missing("--node-name"))?,
        description: description.ok_or(ArgsError::Missing("--description"))?,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_forms() {
        let separated = parse_args(["--node-name", "jieli_test", "--description", "Test"]).unwrap();
        assert_eq!(
            separated,
            PublisherArgs {
                node_name: "jieli_test".to_string(),
                description: "Test".to_string()
            }
        );

        let attached = parse_args(["--node-name=jieli_test", "--description=Test"]).unwrap();
        assert_eq!(attached, separated);
    }

    #[test]
    fn requires_both_options() {
        assert_eq!(
            parse_args(["--node-name", "jieli_test"]),
            Err(ArgsError::Missing("--description"))
        );
        assert_eq!(
            parse_args(["--description", "Test"]),
            Err(ArgsError::Missing("--node-name"))
        );
        assert_eq!(
            parse_args(["--node-name", "--description", "Test"]),
            Err(ArgsError::MissingValue("--node-name".to_string()))
        );
    }

    #[test]
    fn rejects_unknown_arguments_and_reports_help() {
        assert_eq!(
            parse_args(["--node-name", "x", "--description", "y", "--extra"]),
            Err(ArgsError::Unknown("--extra".to_string()))
        );
        assert_eq!(parse_args(["--help"]), Err(ArgsError::Help));
    }
}
