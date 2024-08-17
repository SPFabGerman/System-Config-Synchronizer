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
    package_list.sort_unstable(); // TODO MAYBE: replace by cleanup_package_list (commands should generally not return duplicates, so this may be unnecessary)
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
#[allow(unused)]
fn cleanup_package_list<T: PartialEq + Ord>(l: &mut Vec<T>) {
    l.sort_unstable();
    l.dedup();
}

#[allow(unused)]
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

fn get_from_table<'a, T: toml::macros::Deserialize<'a>>(table: &Table, key: &str, default: T) -> Result<T, Error> {
    table
        .get(key)
        .map_or(Ok(default), |v: &Value| Value::try_into::<T>(v.clone()))
}

pub trait SystemConfigSynchronizer {
    fn get_up_cmds(&self) -> AResult<Vec<CommandVector>>;
    fn get_down_cmds(&self) -> AResult<Vec<CommandVector>>;
}

#[derive(Debug, Clone)]
pub struct PackageSynchronizer {
    packages: Vec<String>,
    meta: PackageSynchronizerMeta,
}

#[derive(Debug, Clone)]
struct PackageSynchronizerMeta {
    installed_packages_cmd: Vec<String>,
    dependency_packages_cmd: Vec<String>,
    explicitly_installed_cmd: Vec<String>,
    explicitly_unrequired_cmd: Vec<String>,
    as_explicit_cmd: Vec<String>,
    install_cmd: Vec<String>,
    as_dependency_cmd: Vec<String>,
    remove_cmd: Vec<String>,
    #[allow(unused)]
    update_cmd: Vec<String>,
    #[allow(unused)]
    get_orphans_cmd: Vec<String>,
    #[allow(unused)]
    get_group_packages_cmd: Vec<String>,
}

pub fn new_pacman(config: &toml::Table) -> AResult<PackageSynchronizer> {
    // Check for unknown keys
    let allowed_keys = ["sudo_cmd", "packages"];
    for k in config.keys() {
        if !allowed_keys.contains(&k.as_str()) {
            return Err(format!("Unknown key: {}", k).into());
        }
    }

    let sudo_cmd = get_from_table(config, "sudo_cmd", "sudo".to_string())?;

    let pacman_config = PackageSynchronizer {
        packages: get_from_table(config, "packages", Vec::new())?,
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

// impl PackageSyncronizer {
//     fn pre_sync(&self) -> AResult<()> {
//         run_cmd(&self.update_cmd)
//     }

//     fn post_sync(&self) -> AResult<()> {
//         let orphans = get_packages_from_command(&self.get_orphans_cmd)?;
//         run_cmd_with_list(&self.remove_cmd, &orphans)
//     }
// }

impl SystemConfigSynchronizer for PackageSynchronizer {
    fn get_up_cmds(&self) -> AResult<Vec<CommandVector>> {
        let config_state = &self.packages;
        let installed_packages = get_packages_from_command(&self.meta.installed_packages_cmd)?;
        let dependency_packages = get_packages_from_command(&self.meta.dependency_packages_cmd)?;

        let to_install = compare_lists_only_in_first(config_state, &installed_packages);
        let to_mark_explicit = compare_lists_in_both(config_state, &dependency_packages);

        let mut cmd_list = Vec::new();

        if !to_mark_explicit.is_empty() {
            let as_explicit_cmd = [self.meta.as_explicit_cmd.clone(), to_mark_explicit].concat();
            cmd_list.push(as_explicit_cmd);
        }
        if !to_install.is_empty() {
            let to_install_cmd = [self.meta.install_cmd.clone(), to_install].concat();
            cmd_list.push(to_install_cmd);
        }

        Ok(cmd_list)
    }

    fn get_down_cmds(&self) -> AResult<Vec<CommandVector>> {
        let config_state = &self.packages;
        let explicitly_installed_packages = get_packages_from_command(&self.meta.explicitly_installed_cmd)?;
        let explicitly_unrequired_packages = get_packages_from_command(&self.meta.explicitly_unrequired_cmd)?;
        let explicitly_required_packages =
            compare_lists_only_in_first(&explicitly_installed_packages, &explicitly_unrequired_packages);

        let to_remove = compare_lists_only_in_first(&explicitly_unrequired_packages, config_state);
        let to_mark_dependency = compare_lists_only_in_first(&explicitly_required_packages, config_state);

        let mut cmd_list = Vec::new();

        if !to_mark_dependency.is_empty() {
            let as_dependency_cmd = [self.meta.as_dependency_cmd.clone(), to_mark_dependency].concat();
            cmd_list.push(as_dependency_cmd);
        }
        if !to_remove.is_empty() {
            let remove_cmd = [self.meta.remove_cmd.clone(), to_remove].concat();
            cmd_list.push(remove_cmd);
        }

        Ok(cmd_list)
    }
}
