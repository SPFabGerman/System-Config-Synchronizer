use std::collections::HashMap;
use std::error::Error;
use std::ffi::OsStr;
use std::fs::{self};
use std::io::{self, BufRead};
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use tera::{Context, Tera};
use toml::Table;

fn run_cmd<T: AsRef<OsStr>>(cmd: &[T]) -> Result<(), Box<dyn Error>> {
    if cmd.is_empty() {
        return Ok(());
    }

    // let cmd_ret = Command::new(&cmd[0]).args(&cmd[1..]).status()?;
    let cmd_ret = Command::new("echo").arg(">").args(cmd).status()?; // For debugging purposes
    if !cmd_ret.success() {
        return Err(Box::from("Command did not succeed"));
    }
    Ok(())
}

fn run_cmd_with_list<T: AsRef<OsStr>>(cmd: &[T], list: &[T]) -> Result<(), Box<dyn Error>> {
    if cmd.is_empty() || list.is_empty() {
        return Ok(());
    }

    // let cmd_ret = Command::new(&cmd[0]).args(&cmd[1..]).args(list).status()?;
    let cmd_ret = Command::new("echo").arg(">").args(cmd).args(list).status()?; // For debugging purposes
    if !cmd_ret.success() {
        return Err(Box::from("Command did not succeed"));
    }
    Ok(())
}

fn get_packages_from_command<T: AsRef<OsStr>>(cmd: &[T]) -> Result<Vec<String>, Box<dyn Error>> {
    if cmd.is_empty() {
        return Err(Box::from("No command was specified!"));
    }

    let mut cmd_proc = Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    if !cmd_proc.wait().expect("Command should be spawned!").success() {
        return Err(Box::from("Command did not succeed"));
    }
    let mut package_list: Vec<String> =
        io::BufReader::new(cmd_proc.stdout.take().expect("Stdout should be available!"))
            .lines()
            .flatten()
            .collect();
    package_list.sort_unstable();
    Ok(package_list)
}

fn get_packages_from_command_with_list<T: AsRef<OsStr>>(cmd: &[T], list: &[T]) -> Result<Vec<String>, Box<dyn Error>> {
    if cmd.is_empty() {
        return Err(Box::from("No command was specified!"));
    }
    if list.is_empty() {
        return Ok(Vec::new());
    }

    let mut cmd_proc = Command::new(&cmd[0])
        .args(&cmd[1..])
        .args(list)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    if !cmd_proc.wait().expect("Command should be spawned!").success() {
        return Err(Box::from("Command did not succeed"));
    }
    let mut package_list: Vec<String> =
        io::BufReader::new(cmd_proc.stdout.take().expect("Stdout should be available!"))
            .lines()
            .flatten()
            .collect();
    package_list.sort_unstable();
    Ok(package_list)
}

fn compare_lists_only_in_first(l1: &[String], l2: &[String]) -> Vec<String> {
    l1.iter()
        .filter(|item| l2.binary_search(item).is_err())
        .cloned()
        .collect()
}

fn compare_lists_in_both(l1: &[String], l2: &[String]) -> Vec<String> {
    l1.iter()
        .filter(|item| l2.binary_search(item).is_ok())
        .cloned()
        .collect()
}

trait SystemConfigSyncronizer {
    type State;
    type UpDiff;
    type DownDiff;
    fn get_current_config_state(&self, path: &Path) -> Result<Self::State, Box<dyn Error>>;
    fn get_current_system_state(&self) -> Result<Self::State, Box<dyn Error>>;
    fn get_up_diff(&self, config_state: &Self::State) -> Result<Self::UpDiff, Box<dyn Error>>;
    fn get_down_diff(&self, config_state: &Self::State) -> Result<Self::DownDiff, Box<dyn Error>>;
    fn report_up_diff(&self, up_diff: &Self::UpDiff);
    fn report_down_diff(&self, down_diff: &Self::DownDiff);
    fn sync_up(&self, up_diff: Self::UpDiff) -> Result<(), Box<dyn Error>>;
    fn sync_down(&self, down_diff: Self::DownDiff) -> Result<(), Box<dyn Error>>;
    fn pre_sync(&self) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
    fn post_sync(&self) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    fn sync_up_down(&self, path: &Path) -> Result<(), Box<dyn Error>> {
        let config_state = self.get_current_config_state(path)?;
        self.pre_sync()?;
        let up_diff = self.get_up_diff(&config_state)?;
        self.report_up_diff(&up_diff);
        self.sync_up(up_diff)?;
        let down_diff = self.get_down_diff(&config_state)?;
        self.report_down_diff(&down_diff);
        self.sync_down(down_diff)?;
        self.post_sync()?;
        Ok(())
    }

    fn sync_down_up(&self, path: &Path) -> Result<(), Box<dyn Error>> {
        let config_state = self.get_current_config_state(path)?;
        self.pre_sync()?;
        let down_diff = self.get_down_diff(&config_state)?;
        self.report_down_diff(&down_diff);
        self.sync_down(down_diff)?;
        let up_diff = self.get_up_diff(&config_state)?;
        self.report_up_diff(&up_diff);
        self.sync_up(up_diff)?;
        self.post_sync()?;
        Ok(())
    }
}

struct ThreeStateSyncronizer {
    current_state_cmd: Vec<String>,
    installed_packages_cmd: Vec<String>,
    dependency_packages_cmd: Vec<String>,
    explicitly_installed_cmd: Vec<String>,
    explicitly_unrequired_cmd: Vec<String>,
    as_explicit_cmd: Vec<String>,
    install_cmd: Vec<String>,
    as_dependency_cmd: Vec<String>,
    remove_cmd: Vec<String>,
    update_cmd: Vec<String>,
    get_orphans_cmd: Vec<String>,
    get_group_packages_cmd: Vec<String>,
    to_install_report_msg: String,
    to_mark_explicit_report_msg: String,
    to_remove_report_msg: String,
    to_mark_dependency_report_msg: String,
}
struct ThreeStateUpDiff {
    to_install: Vec<String>,
    to_mark_explicit: Vec<String>,
}
struct ThreeStateDownDiff {
    to_remove: Vec<String>,
    to_mark_dependency: Vec<String>,
}

fn pacman_default_config() -> ThreeStateSyncronizer {
    ThreeStateSyncronizer {
        current_state_cmd: vec!["pacman".to_string(), "-Qnq".to_string()],
        installed_packages_cmd: vec!["pacman".to_string(), "-Qnq".to_string()],
        dependency_packages_cmd: vec!["pacman".to_string(), "-Qnqd".to_string()],
        explicitly_installed_cmd: vec!["pacman".to_string(), "-Qnqe".to_string()],
        explicitly_unrequired_cmd: vec!["pacman".to_string(), "-Qnqet".to_string()],
        as_explicit_cmd: vec![
            "doas".to_string(),
            "pacman".to_string(),
            "-D".to_string(),
            "--asexplicit".to_string(),
        ],
        install_cmd: vec!["doas".to_string(), "pacman".to_string(), "-S".to_string()],
        as_dependency_cmd: vec![
            "doas".to_string(),
            "pacman".to_string(),
            "-D".to_string(),
            "--asdeps".to_string(),
        ],
        remove_cmd: vec!["doas".to_string(), "pacman".to_string(), "-Rs".to_string()],
        update_cmd: vec!["doas".to_string(), "pacman".to_string(), "-Syu".to_string()],
        get_orphans_cmd: vec!["pacman".to_string(), "-Qnqdt".to_string()],
        get_group_packages_cmd: vec!["pacman".to_string(), "-Sqg".to_string()],
        to_install_report_msg: "Packages to install:".to_string(),
        to_mark_explicit_report_msg: "Packages to mark as explicit:".to_string(),
        to_remove_report_msg: "Packages to remove:".to_string(),
        to_mark_dependency_report_msg: "Packages to mark as dependencies:".to_string(),
    }
}

fn toml_value_to_cmd_array(val: &toml::Value) -> Result<Vec<String>, Box<dyn Error>> {
    match val {
        toml::Value::String(s) => Ok(s.split_whitespace().map(String::from).collect()),
        toml::Value::Array(arr) => {
            let mut str_arr = Vec::new();
            for v in arr {
                match v {
                    toml::Value::String(s) => str_arr.push(s.clone()),
                    _ => return Err("Array contains non-String Elements.".into()),
                }
            }
            Ok(str_arr)
        }
        _ => Err("Value is not String or Array!".into()),
    }
}

fn new_pacman(config: &toml::Table) -> Result<ThreeStateSyncronizer, Box<dyn Error>> {
    let mut pacman_config = pacman_default_config();

    for (k, v) in config {
        match k.as_str() {
            "current_state_cmd" => pacman_config.current_state_cmd = toml_value_to_cmd_array(v)?,
            "installed_packages_cmd" => pacman_config.installed_packages_cmd = toml_value_to_cmd_array(v)?,
            "dependency_packages_cmd" => pacman_config.dependency_packages_cmd = toml_value_to_cmd_array(v)?,
            "explicitly_installed_cmd" => pacman_config.explicitly_installed_cmd = toml_value_to_cmd_array(v)?,
            "explicitly_unrequired_cmd" => pacman_config.explicitly_unrequired_cmd = toml_value_to_cmd_array(v)?,
            "as_explicit_cmd" => pacman_config.as_explicit_cmd = toml_value_to_cmd_array(v)?,
            "install_cmd" => pacman_config.install_cmd = toml_value_to_cmd_array(v)?,
            "as_dependency_cmd" => pacman_config.as_dependency_cmd = toml_value_to_cmd_array(v)?,
            "remove_cmd" => pacman_config.remove_cmd = toml_value_to_cmd_array(v)?,
            "update_cmd" => pacman_config.update_cmd = toml_value_to_cmd_array(v)?,
            "get_orphans_cmd" => pacman_config.get_orphans_cmd = toml_value_to_cmd_array(v)?,
            "get_group_packages_cmd" => pacman_config.get_group_packages_cmd = toml_value_to_cmd_array(v)?,
            "to_install_report_msg" => {
                pacman_config.to_install_report_msg = v.as_str().ok_or("Value is not a String!")?.to_string()
            }
            "to_mark_explicit_report_msg" => {
                pacman_config.to_mark_explicit_report_msg = v.as_str().ok_or("Value is not a String!")?.to_string()
            }
            "to_remove_report_msg" => {
                pacman_config.to_remove_report_msg = v.as_str().ok_or("Value is not a String!")?.to_string()
            }
            "to_mark_dependency_report_msg" => {
                pacman_config.to_mark_dependency_report_msg = v.as_str().ok_or("Value is not a String!")?.to_string()
            }
            _ => eprintln!("Ignoring unknown key: {}", k),
        }
    }

    Ok(pacman_config)
}

impl ThreeStateSyncronizer {
    fn get_group_packages(
        cmd: &[String],
        args: &HashMap<std::string::String, tera::Value>,
    ) -> Result<tera::Value, tera::Error> {
        let groupname = match args.get("name") {
            Some(val) => val.clone(),
            None => return Err("No group was specified".into()),
        };
        let groupname = match groupname {
            tera::Value::String(s) => s,
            _ => return Err("Groupname is no string!".into()),
        };
        let mut packages = match get_packages_from_command_with_list(cmd, &[groupname]) {
            Ok(p) => p,
            Err(_) => return Err("Packages in group could not be found.".into()),
        };

        if let Some(v) = args.get("except") {
            let to_unlist = match v.clone() {
                tera::Value::String(s) => vec![s],
                tera::Value::Array(arr) => {
                    let mut a = Vec::new();
                    for v in arr {
                        match v {
                            tera::Value::String(s) => a.push(s),
                            _ => return Err("Array does contain non-String elements!".into()),
                        }
                    }
                    a
                }
                _ => return Err("except-Keyword can only contain Strings or Array of Strings.".into()),
            };
            packages = compare_lists_only_in_first(&packages, &to_unlist);
        }

        Ok(packages.join("\n").into())
    }
}

impl SystemConfigSyncronizer for ThreeStateSyncronizer {
    type State = Vec<String>;
    type UpDiff = ThreeStateUpDiff;
    type DownDiff = ThreeStateDownDiff;

    fn get_current_config_state(&self, path: &Path) -> Result<Self::State, Box<dyn Error>> {
        // Initialize Tera and load config file from path
        let mut tera = Tera::default();
        tera.add_template_file(path, Some("config_file"))?;

        // Setup functions and variables
        let context = Context::new();

        // Command needs to be evaluated here and not in closure, since the typechecker can't gurantee the closure is only called here.
        // (Which is wierd, since the tera variable drops at the end of the method.)
        // Otherwise we would move part of self out of this method body.
        // We also need to clone the command, since otherwise we would be borrowing out of self, outside this method.
        let cmd = self.get_group_packages_cmd.clone();
        tera.register_function(
            "group",
            Box::new(
                move |args: &HashMap<std::string::String, tera::Value>| -> Result<tera::Value, tera::Error> {
                    ThreeStateSyncronizer::get_group_packages(&cmd, args)
                },
            ),
        );

        // Read config file and map to array
        let render = tera.render("config_file", &context)?;
        let mut package_list: Vec<String> = render.lines().map(String::from).collect();
        package_list.sort_unstable();

        Ok(package_list)
    }

    fn get_current_system_state(&self) -> Result<Self::State, Box<dyn Error>> {
        get_packages_from_command(&self.current_state_cmd)
    }

    fn get_up_diff(&self, config_state: &Self::State) -> Result<ThreeStateUpDiff, Box<dyn Error>> {
        let installed_packages = get_packages_from_command(&self.installed_packages_cmd)?;
        let dependency_packages = get_packages_from_command(&self.dependency_packages_cmd)?;

        Ok(ThreeStateUpDiff {
            to_install: compare_lists_only_in_first(config_state, &installed_packages),
            to_mark_explicit: compare_lists_in_both(config_state, &dependency_packages),
        })
    }

    fn get_down_diff(&self, config_state: &Self::State) -> Result<ThreeStateDownDiff, Box<dyn Error>> {
        let explicitly_installed_packages = get_packages_from_command(&self.explicitly_installed_cmd)?;
        let explicitly_unrequired_packages = get_packages_from_command(&self.explicitly_unrequired_cmd)?;
        let explicitly_required_packages =
            compare_lists_only_in_first(&explicitly_installed_packages, &explicitly_unrequired_packages);

        Ok(ThreeStateDownDiff {
            to_remove: compare_lists_only_in_first(&explicitly_unrequired_packages, config_state),
            to_mark_dependency: compare_lists_only_in_first(&explicitly_required_packages, config_state),
        })
    }

    fn report_up_diff(&self, up_diff: &Self::UpDiff) {
        if !up_diff.to_install.is_empty() {
            println!("{} {}", self.to_install_report_msg, up_diff.to_install.join(", "));
        }
        if !up_diff.to_mark_explicit.is_empty() {
            println!(
                "{} {}",
                self.to_mark_explicit_report_msg,
                up_diff.to_mark_explicit.join(", ")
            );
        }
    }

    fn report_down_diff(&self, down_diff: &Self::DownDiff) {
        if !down_diff.to_remove.is_empty() {
            println!("{} {}", self.to_remove_report_msg, down_diff.to_remove.join(", "));
        }
        if !down_diff.to_mark_dependency.is_empty() {
            println!(
                "{} {}",
                self.to_mark_dependency_report_msg,
                down_diff.to_mark_dependency.join(", ")
            );
        }
    }

    fn sync_up(&self, up_diff: Self::UpDiff) -> Result<(), Box<dyn Error>> {
        run_cmd_with_list(&self.as_explicit_cmd, &up_diff.to_mark_explicit)?;
        run_cmd_with_list(&self.install_cmd, &up_diff.to_install)?;
        Ok(())
    }

    fn sync_down(&self, down_diff: Self::DownDiff) -> Result<(), Box<dyn Error>> {
        run_cmd_with_list(&self.as_dependency_cmd, &down_diff.to_mark_dependency)?;
        run_cmd_with_list(&self.remove_cmd, &down_diff.to_remove)?;
        Ok(())
    }

    fn pre_sync(&self) -> Result<(), Box<dyn Error>> {
        run_cmd(&self.update_cmd)
    }

    fn post_sync(&self) -> Result<(), Box<dyn Error>> {
        let orphans = get_packages_from_command(&self.get_orphans_cmd)?;
        run_cmd_with_list(&self.remove_cmd, &orphans)
    }
}

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
    let config = match fs::read_to_string("config.toml") {
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

    let pacman_config = match config.get("pacman") {
        Some(toml::Value::Table(x)) => x,
        _ => {
            eprintln!("Could not find valid pacman configuration.");
            return ExitCode::FAILURE;
        }
    };

    let pacman_config = match new_pacman(pacman_config) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error in Pacman Config: {}", error_pretty_print(e.as_ref(), false));
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = pacman_config.sync_up_down(Path::new("current_packages")) {
        eprintln!("Error syncronizing: {}", error_pretty_print(e.as_ref(), false));
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
