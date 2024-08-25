use std::error::Error;
use std::fs::{self};
use std::process::Command;
use std::process::ExitCode;
use toml::Table;

pub type AResult<T> = Result<T, Box<dyn Error>>;
pub type CommandVector = Vec<String>;

mod package_synchronizer;
use package_synchronizer::*;

#[allow(unused)]
fn run_cmd(cmd: &[String]) -> AResult<()> {
    if cmd.is_empty() {
        return Ok(());
    }

    let cmd_ret = Command::new(&cmd[0]).args(&cmd[1..]).status()?;
    if !cmd_ret.success() {
        return Err(Box::from("Command did not succeed"));
    }
    Ok(())
}

fn pretty_print_cmds(cmd: &Vec<CommandVector>) {
    for c in cmd {
        println!("> {}", c.join(" "));
    }
}

fn error_pretty_format(err: &dyn Error, skip_first: bool) -> String {
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

fn find_config_tables(table: Table) -> Vec<Table> {
    if table.contains_key("type") {
        return vec![table];
    }

    let mut arr = Vec::new();
    for (_key, value) in table {
        match value {
            toml::Value::Table(subtable) => {
                let new_arr = find_config_tables(subtable);
                arr.extend(new_arr);
            }
            _ => continue,
        }
    }

    arr
}

fn main() -> ExitCode {
    let config_path = "config.toml".to_string();

    let config = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error reading config file: {}", error_pretty_format(&e, false));
            return ExitCode::FAILURE;
        }
    };

    let config = match config.parse::<Table>() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error reading config file: {}", error_pretty_format(&e, false));
            return ExitCode::FAILURE;
        }
    };

    let config_tables = find_config_tables(config);
    let pacman_config = match config_tables.first() {
        Some(x) => x,
        _ => {
            eprintln!("Could not find valid pacman configuration.");
            return ExitCode::FAILURE;
        }
    };

    let pacman_config = match new_pacman(pacman_config) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error in Pacman Config: {}", error_pretty_format(e.as_ref(), false));
            return ExitCode::FAILURE;
        }
    };
    println!("Pacman Config: {:?}", pacman_config);

    let pre_cmds = match pacman_config.get_pre_cmds() {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "Error running query commands: {}",
                error_pretty_format(e.as_ref(), false)
            );
            return ExitCode::FAILURE;
        }
    };
    println!("Pre Commands:");
    pretty_print_cmds(&pre_cmds);

    let up_cmds = match pacman_config.get_up_cmds() {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "Error running query commands: {}",
                error_pretty_format(e.as_ref(), false)
            );
            return ExitCode::FAILURE;
        }
    };
    println!("Up Commands:");
    pretty_print_cmds(&up_cmds);

    let down_cmds = match pacman_config.get_down_cmds() {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "Error running query commands: {}",
                error_pretty_format(e.as_ref(), false)
            );
            return ExitCode::FAILURE;
        }
    };
    println!("Down Commands:");
    pretty_print_cmds(&down_cmds);

    let post_cmds = match pacman_config.get_post_cmds() {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "Error running query commands: {}",
                error_pretty_format(e.as_ref(), false)
            );
            return ExitCode::FAILURE;
        }
    };
    println!("Post Commands:");
    pretty_print_cmds(&post_cmds);

    ExitCode::SUCCESS
}
