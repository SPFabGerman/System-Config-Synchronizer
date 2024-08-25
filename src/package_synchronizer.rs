use crate::{AResult, CommandVector};

use std::ffi::OsStr;
use std::io::{self, BufRead};
use std::process::{Command, Stdio};
use toml::de::Error;
use toml::{Table, Value};

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
    package_list.sort_unstable(); // TODO MAYBE: replace by cleanup_package_list (commands should generally not return duplicates, so this may be unnecessary) or remove
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

#[allow(unused)]
fn toml_value_to_cmd_array(val: &toml::Value) -> AResult<CommandVector> {
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

fn get_from_table<'a, T: toml::macros::Deserialize<'a>>(table: &Table, key: &str, default: T) -> Result<T, Error> {
    table
        .get(key)
        .map_or(Ok(default), |v: &Value| Value::try_into::<T>(v.clone()))
}

/// Single Ok.
/// Convenience wrapper to change one element into a Result+Vector combo with just this element.
/// Always returns `Ok(...)`.
#[allow(non_snake_case)]
fn SOk<T>(element: T) -> AResult<Vec<T>> {
    Ok(vec![element])
}

/// Convenience wrapper to concatenate two lists.
/// All elements are cloned.
fn concat<T: Clone>(l1: &[T], l2: &[T]) -> Vec<T> {
    [l1, l2].concat()
}

pub trait SystemConfigSynchronizer {
    fn get_pre_cmds(&self) -> AResult<Vec<CommandVector>>;
    fn get_post_cmds(&self) -> AResult<Vec<CommandVector>>;
    fn get_up_cmds(&self) -> AResult<Vec<CommandVector>>;
    fn get_down_cmds(&self) -> AResult<Vec<CommandVector>>;
}

#[derive(Debug, Clone)]
pub struct PackageSynchronizer {
    packages: Vec<String>,
    groups: Vec<String>,
    blacklist: Vec<String>,
    meta: PackageSynchronizerMeta,
}

#[derive(Debug, Clone)]
struct PackageSynchronizerMeta {
    installed_packages_cmd: CommandVector,
    dependency_packages_cmd: CommandVector,
    explicitly_installed_cmd: CommandVector,
    explicitly_unrequired_cmd: CommandVector,
    as_explicit_cmd: CommandVector,
    install_cmd: CommandVector,
    as_dependency_cmd: CommandVector,
    remove_cmd: CommandVector,
    update_cmd: CommandVector,
    get_orphans_cmd: CommandVector,
    get_group_packages_cmd: CommandVector,
}

pub fn new_pacman(config: &toml::Table) -> AResult<PackageSynchronizer> {
    let allowed_keys = ["type", "sudo_cmd", "packages", "groups", "blacklist"];

    // Check for unknown keys
    for k in config.keys() {
        if !allowed_keys.contains(&k.as_str()) {
            return Err(format!("Unknown key: {}", k).into());
        }
    }

    let sudo_cmd = get_from_table(config, "sudo_cmd", "sudo".to_string())?;

    let pacman_config = PackageSynchronizer {
        packages: get_from_table(config, "packages", Vec::new())?,
        groups: get_from_table(config, "groups", Vec::new())?,
        blacklist: get_from_table(config, "blacklist", Vec::new())?,
        meta: PackageSynchronizerMeta {
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
        },
    };

    Ok(pacman_config)
}

impl PackageSynchronizer {
    fn calculate_config_state(&self) -> AResult<Vec<String>> {
        // Check if packages and blacklist have an overlap. Error if so.
        let conflicts = compare_lists_in_both(&self.packages, &self.blacklist);
        if !conflicts.is_empty() {
            return Err(format!("Packages and Blacklist have an overlap: {}", conflicts.join(", ")).into());
        }

        let mut config_state = self.packages.clone();
        if !self.groups.is_empty() {
            // Create cmd array
            let mut cmd = self.meta.get_group_packages_cmd.clone();
            cmd.extend(self.groups.clone());
            // Get all packages in the groups
            let group_packages = get_packages_from_command(&cmd)?;
            // Add the group packages to the config state
            config_state.extend(group_packages);
            // Remove all blacklisted packages
            config_state = compare_lists_only_in_first(&config_state, &self.blacklist);
        }

        cleanup_package_list(&mut config_state);
        Ok(config_state)
    }
}

impl SystemConfigSynchronizer for PackageSynchronizer {
    fn get_pre_cmds(&self) -> AResult<Vec<CommandVector>> {
        SOk(self.meta.update_cmd.clone())
    }

    fn get_post_cmds(&self) -> AResult<Vec<CommandVector>> {
        let orphans = get_packages_from_command(&self.meta.get_orphans_cmd)?;
        SOk(concat(&self.meta.remove_cmd, &orphans))
    }

    fn get_up_cmds(&self) -> AResult<Vec<CommandVector>> {
        let config_state = self.calculate_config_state()?;
        let installed_packages = get_packages_from_command(&self.meta.installed_packages_cmd)?;
        let dependency_packages = get_packages_from_command(&self.meta.dependency_packages_cmd)?;

        let to_install = compare_lists_only_in_first(&config_state, &installed_packages);
        let to_mark_explicit = compare_lists_in_both(&config_state, &dependency_packages);

        let mut cmd_list = Vec::new();

        if !to_mark_explicit.is_empty() {
            let as_explicit_cmd = concat(&self.meta.as_explicit_cmd, &to_mark_explicit);
            cmd_list.push(as_explicit_cmd);
        }
        if !to_install.is_empty() {
            let to_install_cmd = concat(&self.meta.install_cmd, &to_install);
            cmd_list.push(to_install_cmd);
        }

        Ok(cmd_list)
    }

    fn get_down_cmds(&self) -> AResult<Vec<CommandVector>> {
        let config_state = self.calculate_config_state()?;
        let explicitly_installed_packages = get_packages_from_command(&self.meta.explicitly_installed_cmd)?;
        let explicitly_unrequired_packages = get_packages_from_command(&self.meta.explicitly_unrequired_cmd)?;
        let explicitly_required_packages =
            compare_lists_only_in_first(&explicitly_installed_packages, &explicitly_unrequired_packages);

        let to_remove = compare_lists_only_in_first(&explicitly_unrequired_packages, &config_state);
        let to_mark_dependency = compare_lists_only_in_first(&explicitly_required_packages, &config_state);

        let mut cmd_list = Vec::new();

        if !to_mark_dependency.is_empty() {
            let as_dependency_cmd = concat(&self.meta.as_dependency_cmd, &to_mark_dependency);
            cmd_list.push(as_dependency_cmd);
        }
        if !to_remove.is_empty() {
            let remove_cmd = concat(&self.meta.remove_cmd, &to_remove);
            cmd_list.push(remove_cmd);
        }

        Ok(cmd_list)
    }
}
