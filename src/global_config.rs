use crate::AResult;

pub struct GlobalConfig {
    pub dry_mode: bool,
}

impl GlobalConfig {
    pub fn default() -> GlobalConfig {
        GlobalConfig { dry_mode: true }
    }

    pub fn new(config: &toml::Table) -> AResult<GlobalConfig> {
        let mut gconfig = GlobalConfig::default();

        for (k, v) in config {
            match k.as_str() {
                "dry_mode" => gconfig.dry_mode = v.as_bool().ok_or("Value is not a Bool!")?,
                _ => {
                    if !v.is_table() {
                        // Ignore tables, since they are not global configurations anymore
                        return Err(format!("Unknown key in global configuration: {}", k).into());
                    }
                }
            }
        }

        Ok(gconfig)
    }
}
