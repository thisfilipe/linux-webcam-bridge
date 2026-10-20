use std::process::ExitCode;

use jieli_install::args::{ArgsError, Mode, parse_args};
use jieli_install::environment::Environment;
use jieli_install::install;
use jieli_install::layout::Layout;

fn main() -> ExitCode {
    let arguments = match parse_args(std::env::args().skip(1)) {
        Ok(arguments) => arguments,
        Err(ArgsError::Help) => {
            println!("{}", jieli_install::args::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("jieli-install: {error}");
            eprintln!("{}", jieli_install::args::USAGE);
            return ExitCode::from(2);
        }
    };

    let environment = match Environment::current() {
        Ok(environment) => environment,
        Err(error) => {
            eprintln!("jieli-install: {error}");
            return ExitCode::FAILURE;
        }
    };
    let layout = match Layout::new(&environment, arguments.plugin.as_deref()) {
        Ok(layout) => layout,
        Err(error) => {
            eprintln!("jieli-install: {error}");
            return ExitCode::FAILURE;
        }
    };

    let result = match arguments.mode {
        Mode::Plan => {
            install::plan(&layout);
            Ok(())
        }
        Mode::Install => install::install(&layout, arguments.replace, true),
        Mode::Remove => install::remove(&layout, true),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("jieli-install: {error}");
            ExitCode::FAILURE
        }
    }
}
