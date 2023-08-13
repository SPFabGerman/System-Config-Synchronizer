use std::collections::HashMap;
use std::error::Error;
use std::ffi::OsStr;
use std::fmt::Display;
use std::fs::{self, File};
use std::io::{self, BufRead, Write};
use std::ops::Not;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use tera::{Context, Tera};
use toml::Table;

type AResult<T> = Result<T, Box<dyn Error>>;

struct GlobalConfig {
    dry_mode: bool,
    show_cmds: bool,
    show_cmds_in_dry_mode: bool,
    show_reports: bool,
    sudo_cmd: String,
    error_on_unknown_keys: bool,
    warn_on_duplicates: bool,
    comment_string: String,
}

impl GlobalConfig {
    fn default() -> GlobalConfig {
        GlobalConfig {
            dry_mode: true,
            show_cmds: true,
            show_cmds_in_dry_mode: true,
            show_reports: true,
            sudo_cmd: "sudo".to_string(),
            error_on_unknown_keys: true,
            warn_on_duplicates: true,
            comment_string: "#".to_string(),
        }
    }

    fn new(config: &toml::Table) -> AResult<GlobalConfig> {
        let mut gconfig = GlobalConfig::default();

        let mut found_unknown_key = false;
        for (k, v) in config {
            match k.as_str() {
                "sudo_cmd" => gconfig.sudo_cmd = v.as_str().ok_or("Value is not a String!")?.to_string(),
                "dry_mode" => gconfig.dry_mode = v.as_bool().ok_or("Value is not a Bool!")?,
                "show_cmds" => gconfig.show_cmds = v.as_bool().ok_or("Value is not a Bool!")?,
                "show_cmds_in_dry_mode" => gconfig.show_cmds_in_dry_mode = v.as_bool().ok_or("Value is not a Bool!")?,
                "show_reports" => gconfig.show_reports = v.as_bool().ok_or("Value is not a Bool!")?,
                "error_on_unknown_keys" => gconfig.error_on_unknown_keys = v.as_bool().ok_or("Value is not a Bool!")?,
                "warn_on_duplicates" => gconfig.warn_on_duplicates = v.as_bool().ok_or("Value is not a Bool!")?,
                "comment_string" => gconfig.comment_string = v.as_str().ok_or("Value is not a String!")?.to_string(),
                _ => {
                    if !v.is_table() {
                        // Ignore tables, since they are not global configurations anymore
                        eprintln!("Unknown key in global configuration: {}", k);
                        found_unknown_key = true;
                    }
                }
            }
        }

        if found_unknown_key {
            if gconfig.error_on_unknown_keys {
                return Err("Usage of unknown keys is not allowed.".into());
            } else {
                eprintln!("Ignoring all unknown keys.");
            }
        }

        Ok(gconfig)
    }
}

fn run_cmd(gconfig: &GlobalConfig, cmd: &[String]) -> AResult<()> {
    if cmd.is_empty() {
        return Ok(());
    }

    if gconfig.show_cmds || (gconfig.dry_mode && gconfig.show_cmds_in_dry_mode) {
        println!("> {}", cmd.join(" "));
    }

    if gconfig.dry_mode {
        return Ok(());
    }

    // let cmd_ret = Command::new(&cmd[0]).args(&cmd[1..]).status()?;
    let cmd_ret = Command::new("echo").arg(">").args(cmd).status()?; // For debugging purposes
    if !cmd_ret.success() {
        return Err(Box::from("Command did not succeed"));
    }
    Ok(())
}

fn run_cmd_with_list(gconfig: &GlobalConfig, cmd: &[String], list: &[String]) -> AResult<()> {
    if cmd.is_empty() || list.is_empty() {
        return Ok(());
    }

    if gconfig.show_cmds || (gconfig.dry_mode && gconfig.show_cmds_in_dry_mode) {
        println!("> {} {}", cmd.join(" "), list.join(" "));
    }

    if gconfig.dry_mode {
        return Ok(());
    }

    // let cmd_ret = Command::new(&cmd[0]).args(&cmd[1..]).args(list).status()?;
    let cmd_ret = Command::new("echo").arg(">").args(cmd).args(list).status()?; // For debugging purposes
    if !cmd_ret.success() {
        return Err(Box::from("Command did not succeed"));
    }
    Ok(())
}

fn get_packages_from_command<T: AsRef<OsStr>>(cmd: &[T]) -> AResult<Vec<String>> {
    if cmd.is_empty() {
        return Ok(Vec::new());
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
    package_list.sort_unstable(); // TODO MAYBE: replace by cleanup_package_list (commands should generally not return duplicates, so this may be unnecessary)
    Ok(package_list)
}

fn get_packages_from_command_with_list<T: AsRef<OsStr>>(cmd: &[T], list: &[T]) -> AResult<Vec<String>> {
    if cmd.is_empty() || list.is_empty() {
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

/// Function that does all the post processing of a package list.
/// Mainly sorting the vector and detecting and removing duplicates.
fn cleanup_package_list<T: PartialEq + Ord + Display>(gconfig: &GlobalConfig, l: &mut Vec<T>) {
    l.sort_unstable();

    // We remove the elements manually, to report found duplicates.
    let mut i = 0;
    while i < l.len() - 1 {
        // Remove all duplicates at once, so the warning is only displayed once.
        // We need to check length again every loop to avoid buffer overflows.
        let mut dupcount = 1;
        while i < l.len() - 1 && l[i] == l[i + 1] {
            l.remove(i + 1);
            dupcount += 1;
        }
        if dupcount > 1 && gconfig.warn_on_duplicates {
            eprintln!("Duplicate element detected: {} (x{})", l[i], dupcount);
        }
        i += 1;
    }
}

fn report(gconfig: &GlobalConfig, msg: &String, objects: &[String], seperator: &str) {
    if !gconfig.show_reports {
        return;
    }

    if !objects.is_empty() {
        println!("{} {}", msg, objects.join(seperator));
    }
}

trait ConfigDiff {
    fn report(&self);
    fn sync(self) -> AResult<()>;
}

trait SystemConfigSyncronizer<'a> {
    type State;
    type UpDiff: ConfigDiff;
    type DownDiff: ConfigDiff;
    fn get_current_config_state(&self) -> AResult<Self::State>;
    fn get_current_system_state(&self) -> AResult<Self::State>;
    fn write_state_to_file(&self, state: &Self::State) -> AResult<()>;
    fn get_up_diff(&'a self, config_state: &Self::State) -> AResult<Self::UpDiff>;
    fn get_down_diff(&'a self, config_state: &Self::State) -> AResult<Self::DownDiff>;
    fn pre_sync(&self) -> AResult<()> {
        Ok(())
    }
    fn post_sync(&self) -> AResult<()> {
        Ok(())
    }

    fn sync_up(&'a self, config_state: &Self::State) -> AResult<()> {
        let up_diff = self.get_up_diff(config_state)?;
        up_diff.report();
        up_diff.sync()?;
        Ok(())
    }

    fn sync_down(&'a self, config_state: &Self::State) -> AResult<()> {
        let down_diff = self.get_down_diff(config_state)?;
        down_diff.report();
        down_diff.sync()?;
        Ok(())
    }

    fn sync_up_only(&'a self) -> AResult<()> {
        self.pre_sync()?;
        let config_state = self.get_current_config_state()?;
        self.sync_up(&config_state)?;
        self.post_sync()?;
        Ok(())
    }

    fn sync_down_only(&'a self) -> AResult<()> {
        self.pre_sync()?;
        let config_state = self.get_current_config_state()?;
        self.sync_down(&config_state)?;
        self.post_sync()?;
        Ok(())
    }

    fn sync_up_down(&'a self) -> AResult<()> {
        self.pre_sync()?;
        let config_state = self.get_current_config_state()?;
        self.sync_up(&config_state)?;
        self.sync_down(&config_state)?;
        self.post_sync()?;
        Ok(())
    }

    fn sync_down_up(&'a self) -> AResult<()> {
        self.pre_sync()?;
        let config_state = self.get_current_config_state()?;
        self.sync_down(&config_state)?;
        self.sync_up(&config_state)?;
        self.post_sync()?;
        Ok(())
    }
}

struct ThreeStateSyncronizer<'a> {
    global_config: &'a GlobalConfig,
    config_path: String,
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
struct ThreeStateUpDiff<'a> {
    parent_sync: &'a ThreeStateSyncronizer<'a>,
    to_install: Vec<String>,
    to_mark_explicit: Vec<String>,
}
struct ThreeStateDownDiff<'a> {
    parent_sync: &'a ThreeStateSyncronizer<'a>,
    to_remove: Vec<String>,
    to_mark_dependency: Vec<String>,
}

fn pacman_default_config(gconfig: &GlobalConfig) -> ThreeStateSyncronizer {
    ThreeStateSyncronizer {
        global_config: gconfig,
        config_path: "current_packages".to_string(),
        installed_packages_cmd: vec!["pacman".to_string(), "-Qnq".to_string()],
        dependency_packages_cmd: vec!["pacman".to_string(), "-Qnqd".to_string()],
        explicitly_installed_cmd: vec!["pacman".to_string(), "-Qnqe".to_string()],
        explicitly_unrequired_cmd: vec!["pacman".to_string(), "-Qnqet".to_string()],
        as_explicit_cmd: vec![
            gconfig.sudo_cmd.clone(),
            "pacman".to_string(),
            "-D".to_string(),
            "--asexplicit".to_string(),
        ],
        install_cmd: vec![gconfig.sudo_cmd.clone(), "pacman".to_string(), "-S".to_string()],
        as_dependency_cmd: vec![
            gconfig.sudo_cmd.clone(),
            "pacman".to_string(),
            "-D".to_string(),
            "--asdeps".to_string(),
        ],
        remove_cmd: vec![gconfig.sudo_cmd.clone(), "pacman".to_string(), "-Rs".to_string()],
        update_cmd: vec![gconfig.sudo_cmd.clone(), "pacman".to_string(), "-Syu".to_string()],
        get_orphans_cmd: vec!["pacman".to_string(), "-Qnqdt".to_string()],
        get_group_packages_cmd: vec!["pacman".to_string(), "-Sqg".to_string()],
        to_install_report_msg: "Packages to install:".to_string(),
        to_mark_explicit_report_msg: "Packages to mark as explicit:".to_string(),
        to_remove_report_msg: "Packages to remove:".to_string(),
        to_mark_dependency_report_msg: "Packages to mark as dependencies:".to_string(),
    }
}

fn toml_value_to_cmd_array(val: &toml::Value) -> AResult<Vec<String>> {
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

fn new_pacman<'a>(gconfig: &'a GlobalConfig, config: &toml::Table) -> AResult<ThreeStateSyncronizer<'a>> {
    let mut pacman_config = pacman_default_config(gconfig);

    let mut found_unknown_key = false;
    for (k, v) in config {
        match k.as_str() {
            "config_path" => pacman_config.config_path = v.as_str().ok_or("Value is not a String!")?.to_string(),
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
            _ => {
                eprintln!("Unknown key: {}", k);
                found_unknown_key = true;
            }
        }
    }

    if found_unknown_key {
        if gconfig.error_on_unknown_keys {
            return Err("Usage of unknown keys is not allowed.".into());
        } else {
            eprintln!("Ignoring all unknown keys.");
        }
    }

    Ok(pacman_config)
}

impl<'a> ThreeStateSyncronizer<'a> {
    fn get_group_packages(
        cmd: &[String],
        args: &HashMap<std::string::String, tera::Value>,
    ) -> core::result::Result<tera::Value, tera::Error> {
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

impl<'a> SystemConfigSyncronizer<'a> for ThreeStateSyncronizer<'a> {
    type State = Vec<String>;
    type UpDiff = ThreeStateUpDiff<'a>;
    type DownDiff = ThreeStateDownDiff<'a>;

    fn get_current_config_state(&self) -> AResult<Self::State> {
        // Initialize Tera and load config file from path
        let mut tera = Tera::default();
        let path = Path::new(&self.config_path);
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
                move |args: &HashMap<std::string::String, tera::Value>| -> core::result::Result<tera::Value, tera::Error> {
                    ThreeStateSyncronizer::get_group_packages(&cmd, args)
                },
            ),
        );

        // Read config file and map to array
        let render = tera.render("config_file", &context)?;
        let mut package_list: Vec<String> = render
            .lines()
            .filter_map(|s| {
                // Remove Comments, whitespace and empty lines
                let s = s
                    .find(&self.global_config.comment_string)
                    .map_or(s, |idx| &s[..idx])
                    .trim();
                s.is_empty().not().then(|| s.to_string())
            })
            .collect();
        cleanup_package_list(self.global_config, &mut package_list);

        Ok(package_list)
    }

    fn get_current_system_state(&self) -> AResult<Self::State> {
        get_packages_from_command(&self.explicitly_installed_cmd)
    }

    fn write_state_to_file(&self, state: &Self::State) -> AResult<()> {
        let path = Path::new(&self.config_path);
        let mut file = File::create(path)?;
        file.write_all(&state.join("\n").into_bytes())?;
        Ok(())
    }

    fn get_up_diff(&'a self, config_state: &Self::State) -> AResult<Self::UpDiff> {
        let installed_packages = get_packages_from_command(&self.installed_packages_cmd)?;
        let dependency_packages = get_packages_from_command(&self.dependency_packages_cmd)?;

        Ok(ThreeStateUpDiff {
            parent_sync: self,
            to_install: compare_lists_only_in_first(config_state, &installed_packages),
            to_mark_explicit: compare_lists_in_both(config_state, &dependency_packages),
        })
    }

    fn get_down_diff(&'a self, config_state: &Self::State) -> AResult<Self::DownDiff> {
        let explicitly_installed_packages = get_packages_from_command(&self.explicitly_installed_cmd)?;
        let explicitly_unrequired_packages = get_packages_from_command(&self.explicitly_unrequired_cmd)?;
        let explicitly_required_packages =
            compare_lists_only_in_first(&explicitly_installed_packages, &explicitly_unrequired_packages);

        Ok(ThreeStateDownDiff {
            parent_sync: self,
            to_remove: compare_lists_only_in_first(&explicitly_unrequired_packages, config_state),
            to_mark_dependency: compare_lists_only_in_first(&explicitly_required_packages, config_state),
        })
    }

    fn pre_sync(&self) -> AResult<()> {
        run_cmd(self.global_config, &self.update_cmd)
    }

    fn post_sync(&self) -> AResult<()> {
        let orphans = get_packages_from_command(&self.get_orphans_cmd)?;
        run_cmd_with_list(self.global_config, &self.remove_cmd, &orphans)
    }
}

impl<'a> ConfigDiff for ThreeStateUpDiff<'a> {
    fn report(&self) {
        report(
            self.parent_sync.global_config,
            &self.parent_sync.to_install_report_msg,
            &self.to_install,
            ", ",
        );
        report(
            self.parent_sync.global_config,
            &self.parent_sync.to_mark_explicit_report_msg,
            &self.to_mark_explicit,
            ", ",
        );
    }

    fn sync(self) -> AResult<()> {
        run_cmd_with_list(
            self.parent_sync.global_config,
            &self.parent_sync.as_explicit_cmd,
            &self.to_mark_explicit,
        )?;
        run_cmd_with_list(
            self.parent_sync.global_config,
            &self.parent_sync.install_cmd,
            &self.to_install,
        )?;
        Ok(())
    }
}

impl<'a> ConfigDiff for ThreeStateDownDiff<'a> {
    fn report(&self) {
        report(
            self.parent_sync.global_config,
            &self.parent_sync.to_remove_report_msg,
            &self.to_remove,
            ", ",
        );
        report(
            self.parent_sync.global_config,
            &self.parent_sync.to_mark_dependency_report_msg,
            &self.to_mark_dependency,
            ", ",
        );
    }

    fn sync(self) -> AResult<()> {
        run_cmd_with_list(
            self.parent_sync.global_config,
            &self.parent_sync.as_dependency_cmd,
            &self.to_mark_dependency,
        )?;
        run_cmd_with_list(
            self.parent_sync.global_config,
            &self.parent_sync.remove_cmd,
            &self.to_remove,
        )?;
        Ok(())
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

    if let Err(e) = pacman_config.sync_up_down() {
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
