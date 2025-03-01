#![cfg_attr(
    not(feature = "full"),
    allow(dead_code, unused_imports, unused_variables)
)]
use std::env;
use std::fs::File;
use std::io::prelude::*;
use std::path::PathBuf;
use thiserror::Error;

pub static GLOBAL_CONFIG_FILE_NAME: &str = if cfg!(target_os = "wasi") {
    "/.private/wapm.toml"
} else {
    "wapm.toml"
};

pub static GLOBAL_CONFIG_FOLDER_NAME: &str = ".wasmer";
pub static GLOBAL_WAX_INDEX_FILE_NAME: &str = ".wax_index.json";
pub static GLOBAL_CONFIG_DATABASE_FILE_NAME: &str = "wapm.sqlite";
pub static GLOBAL_CONFIG_FOLDER_ENV_VAR: &str = "WASMER_DIR";

#[derive(Deserialize, Serialize, Debug, PartialEq)]
pub struct Config {
    /// The number of seconds to wait before checking the registry for a new
    /// version of the package.
    #[serde(default = "wax_default_cooldown")]
    pub wax_cooldown: i32,

    /// The registry that wapm will connect to.
    pub registry: Registry,

    /// Whether or not telemetry is enabled.
    #[cfg(feature = "telemetry")]
    #[serde(default)]
    pub telemetry: Telemetry,

    /// Whether or not updated notifications are enabled.
    #[cfg(feature = "update-notifications")]
    #[serde(default)]
    pub update_notifications: UpdateNotifications,

    /// The proxy to use when connecting to the Internet.
    #[serde(default)]
    pub proxy: Proxy,
}

/// The default cooldown for wax.
pub const fn wax_default_cooldown() -> i32 {
    5 * 60
}

#[derive(Deserialize, Serialize, Debug, PartialEq)]
pub struct Registry {
    pub url: String,
    pub token: Option<String>,
}

#[cfg(feature = "telemetry")]
#[derive(Deserialize, Serialize, Debug, PartialEq)]
pub struct Telemetry {
    pub enabled: String,
}

#[cfg(feature = "telemetry")]
impl Default for Telemetry {
    fn default() -> Telemetry {
        Telemetry {
            enabled: "true".to_string(),
        }
    }
}

#[cfg(feature = "update-notifications")]
#[derive(Deserialize, Serialize, Debug, PartialEq)]
pub struct UpdateNotifications {
    pub enabled: String,
}

#[cfg(feature = "update-notifications")]
impl Default for UpdateNotifications {
    fn default() -> UpdateNotifications {
        Self {
            enabled: "true".to_string(),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, PartialEq, Default)]
pub struct Proxy {
    pub url: Option<String>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            registry: Registry {
                url: "https://registry.wapm.io".to_string(),
                token: None,
            },
            #[cfg(feature = "telemetry")]
            telemetry: Telemetry::default(),
            #[cfg(feature = "update-notifications")]
            update_notifications: UpdateNotifications::default(),
            proxy: Proxy::default(),
            wax_cooldown: wax_default_cooldown(),
        }
    }
}

impl Config {
    pub fn get_current_dir() -> std::io::Result<PathBuf> {
        #[cfg(target_os = "wasi")]
        if let Some(pwd) = std::env::var("PWD").ok() {
            return Ok(PathBuf::from(pwd));
        }
        Ok(std::env::current_dir()?)
    }

    pub fn get_folder() -> Result<PathBuf, GlobalConfigError> {
        Ok(
            if let Some(folder_str) = env::var(GLOBAL_CONFIG_FOLDER_ENV_VAR)
                .ok()
                .filter(|s| !s.is_empty())
            {
                PathBuf::from(folder_str)
            } else {
                #[allow(unused_variables)]
                let default_dir = Self::get_current_dir()
                    .ok()
                    .unwrap_or_else(|| PathBuf::from("/".to_string()));
                #[cfg(feature = "dirs")]
                let home_dir =
                    dirs::home_dir().ok_or(GlobalConfigError::CannotFindHomeDirectory)?;
                #[cfg(not(feature = "dirs"))]
                let home_dir = std::env::var("HOME")
                    .ok()
                    .unwrap_or_else(|| default_dir.to_string_lossy().to_string());
                let mut folder = PathBuf::from(home_dir);
                folder.push(GLOBAL_CONFIG_FOLDER_NAME);
                std::fs::create_dir_all(folder.clone())
                    .map_err(|e| GlobalConfigError::CannotCreateConfigDirectory(e))?;
                folder
            },
        )
    }

    fn get_file_location() -> Result<PathBuf, GlobalConfigError> {
        Ok(Self::get_folder()?.join(GLOBAL_CONFIG_FILE_NAME))
    }

    pub fn get_wax_file_path() -> Result<PathBuf, GlobalConfigError> {
        Config::get_folder().map(|config_folder| config_folder.join(GLOBAL_WAX_INDEX_FILE_NAME))
    }

    pub fn get_database_file_path() -> Result<PathBuf, GlobalConfigError> {
        Config::get_folder()
            .map(|config_folder| config_folder.join(GLOBAL_CONFIG_DATABASE_FILE_NAME))
    }

    /// Load the config from a file
    #[cfg(not(feature = "integration_tests"))]
    pub fn from_file() -> Result<Self, GlobalConfigError> {
        let path = Self::get_file_location()?;
        match File::open(&path) {
            Ok(mut file) => {
                let mut config_toml = String::new();
                file.read_to_string(&mut config_toml)
                    .map_err(|e| GlobalConfigError::Io(e))?;
                toml::from_str(&config_toml).map_err(|e| GlobalConfigError::Toml(e))
            }
            Err(_e) => Ok(Self::default()),
        }
    }

    /// A mocked version of the standard function for integration tests
    #[cfg(feature = "integration_tests")]
    pub fn from_file() -> Result<Self, GlobalConfigError> {
        crate::integration_tests::data::RAW_CONFIG_DATA.with(|rcd| {
            if let Some(ref config_toml) = *rcd.borrow() {
                toml::from_str(&config_toml).map_err(|e| GlobalConfigError::Toml(e))
            } else {
                Ok(Self::default())
            }
        })
    }

    pub fn get_globals_directory() -> Result<PathBuf, GlobalConfigError> {
        Self::get_folder().map(|p| p.join("globals"))
    }

    /// Save the config to a file
    #[cfg(not(feature = "integration_tests"))]
    pub fn save(self: &Self) -> anyhow::Result<()> {
        let path = Self::get_file_location()?;
        let config_serialized = toml::to_string(&self)?;
        let mut file = File::create(path)?;
        file.write_all(config_serialized.as_bytes())?;
        Ok(())
    }

    /// A mocked version of the standard function for integration tests
    #[cfg(feature = "integration_tests")]
    pub fn save(self: &Self) -> anyhow::Result<()> {
        let config_serialized = toml::to_string(&self)?;
        crate::integration_tests::data::RAW_CONFIG_DATA.with(|rcd| {
            *rcd.borrow_mut() = Some(config_serialized);
        });

        Ok(())
    }

    #[cfg(feature = "update-notifications")]
    pub fn update_notifications_enabled() -> bool {
        Self::from_file()
            .map(|c| c.update_notifications.enabled == "true")
            .unwrap_or(true)
    }
}

impl Registry {
    pub fn get_graphql_url(self: &Self) -> String {
        let url = &self.url;
        if url.ends_with("/") {
            format!("{}graphql", url)
        } else {
            format!("{}/graphql", url)
        }
    }
}

#[derive(Debug, Error)]
pub enum GlobalConfigError {
    #[error("Error while reading config: [{0}]")]
    Io(std::io::Error),
    #[error("Error while reading config: [{0}]")]
    Toml(toml::de::Error),
    #[error(
        "While falling back to the default location for WASMER_DIR, could not resolve the user's home directory"
    )]
    CannotFindHomeDirectory,
    #[error("Error while creating config directory: [{0}]")]
    CannotCreateConfigDirectory(std::io::Error),
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Key not found: {key}")]
    KeyNotFound { key: String },
    #[error("Failed to parse value `{value}` for key `{key}`")]
    CanNotParse { value: String, key: String },
}

pub fn set(config: &mut Config, key: String, value: String) -> anyhow::Result<()> {
    match key.as_ref() {
        "registry.url" => {
            if config.registry.url != value {
                config.registry.url = value;
                // Resets the registry token automatically
                config.registry.token = None;
            }
        }
        "registry.token" => {
            config.registry.token = Some(value);
        }
        #[cfg(feature = "telemetry")]
        "telemetry.enabled" => {
            config.telemetry.enabled = value;
        }
        #[cfg(feature = "update-notifications")]
        "update-notifications.enabled" => {
            config.update_notifications.enabled = value;
        }
        "proxy.url" => {
            config.proxy.url = if value.is_empty() { None } else { Some(value) };
        }
        "wax.cooldown" => {
            let num = value.parse::<i32>().map_err(|_| ConfigError::CanNotParse {
                value: value.clone(),
                key: key.clone(),
            })?;
            config.wax_cooldown = num;
        }
        _ => {
            return Err(ConfigError::KeyNotFound { key }.into());
        }
    };
    config.save()?;
    Ok(())
}

pub fn get(config: &mut Config, key: String) -> anyhow::Result<String> {
    let value = match key.as_ref() {
        "registry.url" => config.registry.url.clone(),
        "registry.token" => {
            unimplemented!()
            // &(config.registry.token.as_ref().map_or("".to_string(), |n| n.to_string()).to_owned())
        }
        #[cfg(feature = "telemetry")]
        "telemetry.enabled" => config.telemetry.enabled.clone(),
        #[cfg(feature = "update-notifications")]
        "update-notifications.enabled" => config.update_notifications.enabled.clone(),
        "proxy.url" => {
            if let Some(url) = &config.proxy.url {
                url.clone()
            } else {
                "No proxy configured".to_owned()
            }
        }
        "wax.cooldown" => format!("{}", config.wax_cooldown),
        _ => {
            return Err(ConfigError::KeyNotFound { key }.into());
        }
    };
    Ok(value)
}

#[cfg(test)]
mod test {
    use crate::config::{Config, GLOBAL_CONFIG_FILE_NAME, GLOBAL_CONFIG_FOLDER_ENV_VAR};
    use crate::util::create_temp_dir;
    use std::fs::*;
    use std::io::Write;

    #[test]
    fn get_config_and_wasmer_dir_does_not_exist() {
        // remove WASMER_DIR
        let _ = std::env::remove_var(GLOBAL_CONFIG_FOLDER_ENV_VAR);
        let config_result = Config::from_file();
        assert!(
            !config_result.is_err(),
            "Config file created by falling back to default"
        );
    }

    #[test]
    fn get_non_existent_config() {
        let tmp_dir = create_temp_dir().unwrap();
        // set the env var to our temp dir
        std::env::set_var(GLOBAL_CONFIG_FOLDER_ENV_VAR, tmp_dir.display().to_string());
        let config_result = Config::from_file();
        assert!(config_result.is_ok(), "Did not find the default config.");
        let actual_config = config_result.unwrap();
        let expected_config = Config::default();
        assert_eq!(
            expected_config, actual_config,
            "Found config is not the default config."
        );
    }

    #[test]
    fn get_global_config() {
        let tmp_dir = create_temp_dir().unwrap();
        let manifest_absolute_path = tmp_dir.join(GLOBAL_CONFIG_FILE_NAME);
        let mut file = File::create(&manifest_absolute_path).unwrap();
        let config = Config::default();
        let config_string = toml::to_string(&config).unwrap();
        file.write_all(config_string.as_bytes()).unwrap();
        // set the env var to our temp dir
        std::env::set_var(GLOBAL_CONFIG_FOLDER_ENV_VAR, tmp_dir.display().to_string());
        let config_result = Config::from_file();
        assert!(config_result.is_ok(), "Config not found.");
    }
}
