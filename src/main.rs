use std::error::Error;
use std::fs::{self};
use std::process::ExitCode;
use toml::Table;

pub type AResult<T> = Result<T, Box<dyn Error>>;

mod global_config;
use global_config::GlobalConfig;

mod package_synchronizer;
use package_synchronizer::*;

fn error_pretty_print(err: &dyn Error, skip_first: bool) -> String {
    let mut skip_first = skip_first;
    let mut s = Vec::new();
    let mut err: Option<&dyn Error> = Some(err);
    while let Some(e) = err {
        if !skip_first {
            s.push(e.to_string());
        }
        skip_first = false;
        err = e.source();
    }

    // If the Error string contains newlines, assume we have a multiline error and display it on its own lines.
    if s.concat().contains('\n') {
        format!("\n{}", s.join("\n"))
    } else {
        s.join(": ")
    }
}

fn main() -> ExitCode {
    let config_path = match std::env::var("SCS_GLOBAL_CONFIG") {
        Ok(p) => p,
        Err(std::env::VarError::NotPresent) => "config.toml".to_string(),
        Err(e) => {
            eprintln!(
                "Error reading environment variable SCS_GLOBAL_CONFIG: {}",
                error_pretty_print(&e, false)
            );
            return ExitCode::FAILURE;
        }
    };

    let config = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error reading config file: {}", error_pretty_print(&e, false));
            return ExitCode::FAILURE;
        }
    };

    let config = match config.parse::<Table>() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error reading config file: {}", error_pretty_print(&e, false));
            return ExitCode::FAILURE;
        }
    };

    let global_config = match GlobalConfig::new(&config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "Error in Global Configuration: {}",
                error_pretty_print(e.as_ref(), false)
            );
            return ExitCode::FAILURE;
        }
    };

    let pacman_config = match config.get("pacman") {
        Some(toml::Value::Table(x)) => x,
        _ => {
            eprintln!("Could not find valid pacman configuration.");
            return ExitCode::FAILURE;
        }
    };

    let pacman_config = match new_pacman(&global_config, pacman_config) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error in Pacman Config: {}", error_pretty_print(e.as_ref(), false));
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = pacman_config.sync() {
        eprintln!("Error syncronizing: {}", error_pretty_print(e.as_ref(), false));
        return ExitCode::FAILURE;
    }

    // let state = match pacman_config.get_current_system_state() {
    //     Ok(s) => s,
    //     Err(e) => {
    //         eprintln!("Error retrieving system state: {}", error_pretty_print(e.as_ref(), false));
    //         return ExitCode::FAILURE;
    //     }
    // };

    // if let Err(e) = pacman_config.write_state_to_file(&state) {
    //     eprintln!("Error writing state to file: {}", error_pretty_print(e.as_ref(), false));
    //     return ExitCode::FAILURE;
    // }

    ExitCode::SUCCESS
}
