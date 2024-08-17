use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::{self, BufRead};
use std::ops::Not;
use std::path::Path;
use std::process::{Command, Stdio};
use tera::{Context, Tera};
use toml::Value;

use crate::AResult;

fn run_cmd(cmd: &[String]) -> AResult<()> {
    if cmd.is_empty() {
        return Ok(());
    }

    println!("> {}", cmd.join(" "));

    // let cmd_ret = Command::new(&cmd[0]).args(&cmd[1..]).status()?;
    let cmd_ret = Command::new("echo").arg(">").args(cmd).status()?; // For debugging purposes
    if !cmd_ret.success() {
        return Err(Box::from("Command did not succeed"));
    }
    Ok(())
}

fn run_cmd_with_list(cmd: &[String], list: &[String]) -> AResult<()> {
    if cmd.is_empty() || list.is_empty() {
        return Ok(());
    }

    println!("> {} {}", cmd.join(" "), list.join(" "));

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
            .map_while(Result::ok)
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
            .map_while(Result::ok)
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
fn cleanup_package_list<T: PartialEq + Ord>(l: &mut Vec<T>) {
    l.sort_unstable();
    l.dedup();
}

fn report(msg: &String, objects: &[String], seperator: &str) {
    if !objects.is_empty() {
        println!("{} {}", msg, objects.join(seperator));
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

pub trait SystemConfigSyncronizer {
    fn sync(self) -> AResult<()>;
}

#[derive(Clone)]
pub struct PackageSyncronizer {
    #[allow(unused)]
    sudo_cmd: String,
    comment_string: String,
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
}
struct ThreeStateUpDiff {
    parent_sync: PackageSyncronizer,
    to_install: Vec<String>,
    to_mark_explicit: Vec<String>,
}
struct ThreeStateDownDiff {
    parent_sync: PackageSyncronizer,
    to_remove: Vec<String>,
    to_mark_dependency: Vec<String>,
}

fn pacman_default_config(sudo_cmd: String) -> PackageSyncronizer {
    PackageSyncronizer {
        comment_string: "#".to_string(),
        config_path: "current_packages".to_string(),
        installed_packages_cmd: vec!["pacman".to_string(), "-Qnq".to_string()],
        dependency_packages_cmd: vec!["pacman".to_string(), "-Qnqd".to_string()],
        explicitly_installed_cmd: vec!["pacman".to_string(), "-Qnqe".to_string()],
        explicitly_unrequired_cmd: vec!["pacman".to_string(), "-Qnqet".to_string()],
        as_explicit_cmd: vec![
            sudo_cmd.clone(),
            "pacman".to_string(),
            "-D".to_string(),
            "--asexplicit".to_string(),
        ],
        install_cmd: vec![sudo_cmd.clone(), "pacman".to_string(), "-S".to_string()],
        as_dependency_cmd: vec![
            sudo_cmd.clone(),
            "pacman".to_string(),
            "-D".to_string(),
            "--asdeps".to_string(),
        ],
        remove_cmd: vec![sudo_cmd.clone(), "pacman".to_string(), "-Rs".to_string()],
        update_cmd: vec![sudo_cmd.clone(), "pacman".to_string(), "-Syu".to_string()],
        get_orphans_cmd: vec!["pacman".to_string(), "-Qnqdt".to_string()],
        get_group_packages_cmd: vec!["pacman".to_string(), "-Sqg".to_string()],
        sudo_cmd, // Move value last, to allow borrows
    }
}

pub fn new_pacman(config: &toml::Table) -> AResult<PackageSyncronizer> {
    let mut pacman_config = pacman_default_config(
        config
            .get("sudo_cmd")
            .unwrap_or(&Value::String("sudo".to_string()))
            .as_str()
            .ok_or("Value is not a String!")?
            .to_string(),
    );

    for (k, v) in config {
        match k.as_str() {
            "config_path" => pacman_config.config_path = v.as_str().ok_or("Value is not a String!")?.to_string(),
            "comment_string" => pacman_config.comment_string = v.as_str().ok_or("Value is not a String!")?.to_string(),
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
            "sudo_cmd" => {}
            _ => {
                return Err(format!("Unknown key: {}", k).into());
            }
        }
    }

    Ok(pacman_config)
}

impl PackageSyncronizer {
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

    fn get_current_config_state(&self) -> AResult<Vec<String>> {
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
                    PackageSyncronizer::get_group_packages(&cmd, args)
                },
            ),
        );

        // Read config file and map to array
        let render = tera.render("config_file", &context)?;
        let mut package_list: Vec<String> = render
            .lines()
            .filter_map(|s| {
                // Remove Comments, whitespace and empty lines
                let s = s.find(&self.comment_string).map_or(s, |idx| &s[..idx]).trim();
                s.is_empty().not().then(|| s.to_string())
            })
            .collect();
        cleanup_package_list(&mut package_list);

        Ok(package_list)
    }

    fn get_up_diff(&self, config_state: &[String]) -> AResult<ThreeStateUpDiff> {
        let installed_packages = get_packages_from_command(&self.installed_packages_cmd)?;
        let dependency_packages = get_packages_from_command(&self.dependency_packages_cmd)?;

        Ok(ThreeStateUpDiff {
            parent_sync: self.clone(),
            to_install: compare_lists_only_in_first(config_state, &installed_packages),
            to_mark_explicit: compare_lists_in_both(config_state, &dependency_packages),
        })
    }

    fn get_down_diff(&self, config_state: &[String]) -> AResult<ThreeStateDownDiff> {
        let explicitly_installed_packages = get_packages_from_command(&self.explicitly_installed_cmd)?;
        let explicitly_unrequired_packages = get_packages_from_command(&self.explicitly_unrequired_cmd)?;
        let explicitly_required_packages =
            compare_lists_only_in_first(&explicitly_installed_packages, &explicitly_unrequired_packages);

        Ok(ThreeStateDownDiff {
            parent_sync: self.clone(),
            to_remove: compare_lists_only_in_first(&explicitly_unrequired_packages, config_state),
            to_mark_dependency: compare_lists_only_in_first(&explicitly_required_packages, config_state),
        })
    }

    fn pre_sync(&self) -> AResult<()> {
        run_cmd(&self.update_cmd)
    }

    fn post_sync(&self) -> AResult<()> {
        let orphans = get_packages_from_command(&self.get_orphans_cmd)?;
        run_cmd_with_list(&self.remove_cmd, &orphans)
    }
}

impl ThreeStateUpDiff {
    fn report(&self) {
        report(&"Packages to install:".to_string(), &self.to_install, ", ");
        report(
            &"Packages to mark as explicit:".to_string(),
            &self.to_mark_explicit,
            ", ",
        );
    }

    fn sync(self) -> AResult<()> {
        run_cmd_with_list(&self.parent_sync.as_explicit_cmd, &self.to_mark_explicit)?;
        run_cmd_with_list(&self.parent_sync.install_cmd, &self.to_install)?;
        Ok(())
    }
}

impl ThreeStateDownDiff {
    fn report(&self) {
        report(&"Packages to remove:".to_string(), &self.to_remove, ", ");
        report(
            &"Packages to mark as dependencies:".to_string(),
            &self.to_mark_dependency,
            ", ",
        );
    }

    fn sync(self) -> AResult<()> {
        run_cmd_with_list(&self.parent_sync.as_dependency_cmd, &self.to_mark_dependency)?;
        run_cmd_with_list(&self.parent_sync.remove_cmd, &self.to_remove)?;
        Ok(())
    }
}

impl SystemConfigSyncronizer for PackageSyncronizer {
    fn sync(self) -> AResult<()> {
        self.pre_sync()?;

        let config_state = self.get_current_config_state()?;

        let up_diff = self.get_up_diff(&config_state)?;
        up_diff.report();
        up_diff.sync()?;

        let down_diff = self.get_down_diff(&config_state)?;
        down_diff.report();
        down_diff.sync()?;

        self.post_sync()?;

        Ok(())
    }
}
