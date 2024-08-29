use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use xshell::Shell;
use url::Url;
use zksync_basic_types::url::SensitiveUrl;

use crate::explorer;
use crate::{
    consts::{APPS_CONFIG_FILE, LOCAL_CHAINS_PATH, LOCAL_CONFIGS_PATH},
    traits::{FileConfigWithDefaultName, ReadConfig, SaveConfig, ZkToolboxConfig},
};

pub const DEFAULT_EXPLORER_PORT: u16 = 3010;
pub const DEFAULT_PORTAL_PORT: u16 = 3030;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppsEcosystemConfig {
    pub portal: AppEcosystemConfig,
    pub explorer: AppEcosystemConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppEcosystemConfig {
    pub http_port: u16,
    pub http_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chains_enabled: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppsChainConfig {
    pub portal: PortalAppChainConfig,
    pub explorer: ExplorerAppChainConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PortalAppChainConfig {
    pub l2_rpc_url: Url,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExplorerAppChainConfig {
    pub l2_rpc_url: Url,
    pub verification_api_url: Option<Url>,
    pub database_url: Option<Url>,
    pub services: Option<ServicesConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServicesConfig {
    pub worker: WorkerConfig,
    pub data_fetcher: DataFetcherConfig,
    pub api: ApiConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkerConfig {
    pub http_port: u16,
    pub batches_processing_polling_interval: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DataFetcherConfig {
    pub http_port: u16,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiConfig {
    pub http_port: u16,
    pub http_url: Url,
    pub metrics_port: u16,
}

impl ZkToolboxConfig for AppsEcosystemConfig {}
impl FileConfigWithDefaultName for AppsEcosystemConfig {
    const FILE_NAME: &'static str = APPS_CONFIG_FILE;
}

impl ZkToolboxConfig for AppsChainConfig {}

impl AppsEcosystemConfig {
    pub fn default() -> Self {
        AppsEcosystemConfig {
            portal: AppEcosystemConfig {
                http_port: DEFAULT_PORTAL_PORT,
                http_url: format!("http://127.0.0.1:{}", DEFAULT_PORTAL_PORT),
                chains_enabled: None,
            },
            explorer: AppEcosystemConfig {
                http_port: DEFAULT_EXPLORER_PORT,
                http_url: format!("http://127.0.0.1:{}", DEFAULT_EXPLORER_PORT),
                chains_enabled: None,
            },
        }
    }

    pub fn get_config_path(ecosystem_base_path: &Path) -> PathBuf {
        ecosystem_base_path
            .join(LOCAL_CONFIGS_PATH)
            .join(APPS_CONFIG_FILE)
    }

    pub fn read_or_create_default(shell: &Shell) -> anyhow::Result<Self> {
        let config_path = Self::get_config_path(&shell.current_dir());
        match Self::read(shell, &config_path) {
            Ok(config) => Ok(config),
            Err(_) => {
                let config = Self::default();
                config.save(shell, &config_path)?;
                Ok(config)
            }
        }
    }

    // pub fn set_chain_enabled(&mut self, chain_name: &str, enable_portal: bool, enable_explorer: bool) {
    //     if enable_portal {
    //         if !self.portal.chains_enabled.contains(&chain_name.to_string()) {
    //             self.portal.chains_enabled.push(chain_name.to_string());
    //         }
    //     } else {
    //         self.portal.chains_enabled.retain(|chain| chain != chain_name);
    //     }

    //     if enable_explorer {
    //         if !self.explorer.chains_enabled.contains(&chain_name.to_string()) {
    //             self.explorer.chains_enabled.push(chain_name.to_string());
    //         }
    //     } else {
    //         self.explorer.chains_enabled.retain(|chain| chain != chain_name);
    //     }
    // }
}

impl AppsChainConfig {
    pub fn new(portal: PortalAppChainConfig, explorer: ExplorerAppChainConfig) -> Self {
        AppsChainConfig {
            portal,
            explorer,
        }
    }

    pub fn default() -> Self {
        AppsChainConfig {
            portal: PortalAppChainConfig {
                l2_rpc_url: Url::parse("http://fuck.com").unwrap(),
            },
            explorer: ExplorerAppChainConfig {
                l2_rpc_url: Url::parse("http://fuck.com").unwrap(),
                database_url: None,
                services: None,
                verification_api_url: None,
            },
        }
    }

    pub fn get_config_path(ecosystem_base_path: &Path, chain_name: &str) -> PathBuf {
        ecosystem_base_path
            .join(LOCAL_CHAINS_PATH)
            .join(chain_name)
            .join(LOCAL_CONFIGS_PATH)
            .join(APPS_CONFIG_FILE)
    }

    pub fn read_or_create_default(shell: &Shell, chain_name: &str) -> anyhow::Result<Self> {
        let config_path = Self::get_config_path(&shell.current_dir(), chain_name);
        match Self::read(shell, &config_path) {
            Ok(config) => Ok(config),
            Err(_) => {
                let config = Self::default();
                config.save(shell, &config_path)?;
                Ok(config)
            }
        }
    }
}