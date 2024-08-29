use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use anyhow::{Result, Context};
use url::Url;

use crate::{
    consts::{EXPLORER_BACKEND_DOCKER_COMPOSE_FILE, EXPLORER_DOCKER_COMPOSE_FILE, LOCAL_APPS_PATH, LOCAL_CHAINS_PATH, LOCAL_CONFIGS_PATH, LOCAL_GENERATED_PATH},
    docker_compose::{DockerComposeConfig, DockerComposeService},
    traits::ZkToolboxConfig,
};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExplorerComposeConfig {
    #[serde(flatten)]
    pub docker_compose: DockerComposeConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExplorerBackendComposeConfig {
    #[serde(flatten)]
    pub docker_compose: DockerComposeConfig,
}

impl ZkToolboxConfig for ExplorerComposeConfig {}
impl ZkToolboxConfig for ExplorerBackendComposeConfig {}

#[derive(Debug, Clone)]
pub struct ExplorerAppServiceConfig {
    pub port: u16,
    pub config_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ExplorerBackendServiceConfig {
    pub db_url: String,
    pub rpc_port: u16,
    pub service_ports: ExplorerBackendServicePorts,
}

#[derive(Debug, Clone)]
pub struct ExplorerBackendServicePorts {
    pub api_port: u16,
    pub data_fetcher_port: u16,
    pub worker_port: u16,
}

impl ExplorerComposeConfig {
    pub fn new(app_config: ExplorerAppServiceConfig, backend_configs: Vec<ExplorerBackendComposeConfig>) -> Result<Self> {
        let mut services = HashMap::new();
        let mut app_depends_on = Vec::new();

        // Add services from backend configs
        for backend_config in backend_configs.iter() {
            for (service_name, service) in &backend_config.docker_compose.services {
                if service.image.contains("block-explorer-api") {
                    app_depends_on.push(service_name.clone());
                }
                services.insert(service_name.clone(), service.clone());
            }
        }

        services.insert(
            "block-explorer-app".to_string(),
            Self::create_app_service(app_config, Some(app_depends_on)),
        );

        let config = Self {
            docker_compose: DockerComposeConfig { services },
        };
        Ok(config)
    }

    fn create_app_service(app_config: ExplorerAppServiceConfig, depends_on: Option<Vec<String>>) -> DockerComposeService {
        DockerComposeService {
            image: "matterlabs/block-explorer-app".to_string(),
            platform: Some("linux/amd64".to_string()),
            ports: Some(vec![format!("{}:3010", app_config.port)]),
            volumes: Some(vec![format!("{}:/usr/src/app/packages/app/dist/config.js", app_config.config_path.display())]),
            depends_on,
            restart: Some("unless-stopped".to_string()),
            environment: None,
            extra_hosts: None,
        }
    }

    pub fn get_config_path(ecosystem_base_path: &Path) -> PathBuf {
        ecosystem_base_path
            .join(LOCAL_CONFIGS_PATH)
            .join(LOCAL_GENERATED_PATH)
            .join(EXPLORER_DOCKER_COMPOSE_FILE)
    }
}

impl ExplorerBackendComposeConfig {
    pub fn new(chain_name: &str, config: ExplorerBackendServiceConfig) -> anyhow::Result<Self> {
        let mut services: HashMap<String, DockerComposeService> = HashMap::new();

        let db_url = Url::parse(&config.db_url).context("Failed to parse database URL")?;
        let db_host = db_url.host_str().context("No host in database URL")?.to_string();
        let db_user = db_url.username().to_string();
        let db_password = db_url.password().context("No password in database URL")?.to_string();
        let db_name = db_url.path().trim_start_matches('/').to_string();

        services.insert(
            format!("block-explorer-api-{}", chain_name),
            Self::create_api_service(chain_name, config.service_ports.api_port, &config.db_url),
        );
        services.insert(
            format!("block-explorer-data-fetcher-{}", chain_name),
            Self::create_data_fetcher_service( config.service_ports.data_fetcher_port, config.rpc_port),
        );
        services.insert(
            format!("block-explorer-worker-{}", chain_name),
            Self::create_worker_service(chain_name, config.service_ports.worker_port, config.rpc_port, &db_host, &db_user, &db_password, &db_name),
        );

        let config = Self {
            docker_compose: DockerComposeConfig { services },
        };
        Ok(config)
    }

    fn create_api_service(chain_name: &str, port: u16, db_url: &str) -> DockerComposeService {
        DockerComposeService {
            image: "matterlabs/block-explorer-api".to_string(),
            platform: Some("linux/amd64".to_string()),
            ports: Some(vec![format!("{}:{}", port, port)]),
            volumes: None,
            depends_on: Some(vec![format!("worker-{}", chain_name)]),
            restart: Some("unless-stopped".to_string()),
            environment: Some(HashMap::from([
                ("PORT".to_string(), port.to_string()),
                ("LOG_LEVEL".to_string(), "verbose".to_string()),
                ("NODE_ENV".to_string(), "development".to_string()),
                ("DATABASE_URL".to_string(), db_url.to_string()),
            ])),
            extra_hosts: Some(vec!["host.docker.internal:host-gateway".to_string()]),
        }
    }

    fn create_data_fetcher_service(port: u16, rpc_port: u16) -> DockerComposeService {
        DockerComposeService {
            image: "matterlabs/block-explorer-data-fetcher".to_string(),
            platform: Some("linux/amd64".to_string()),
            ports: Some(vec![format!("{}:{}", port, port)]),
            volumes: None,
            depends_on: None,
            restart: Some("unless-stopped".to_string()),
            environment: Some(HashMap::from([
                ("PORT".to_string(), port.to_string()),
                ("LOG_LEVEL".to_string(), "verbose".to_string()),
                ("NODE_ENV".to_string(), "development".to_string()),
                ("BLOCKCHAIN_RPC_URL".to_string(), format!("http://host.docker.internal:{}", rpc_port)),
            ])),
            extra_hosts: Some(vec!["host.docker.internal:host-gateway".to_string()]),
        }
    }

    fn create_worker_service(
        chain_name: &str,
        port: u16,
        rpc_port: u16,
        db_host: &str,
        db_user: &str,
        db_password: &str,
        db_name: &str,
    ) -> DockerComposeService {
        let data_fetcher_url = format!("http://data-fetcher-{}:{}", chain_name, port);
        DockerComposeService {
            image: "matterlabs/block-explorer-worker".to_string(),
            platform: Some("linux/amd64".to_string()),
            ports: None,
            volumes: None,
            depends_on: None,
            restart: Some("unless-stopped".to_string()),
            environment: Some(HashMap::from([
                ("PORT".to_string(), port.to_string()),
                ("LOG_LEVEL".to_string(), "verbose".to_string()),
                ("NODE_ENV".to_string(), "development".to_string()),
                ("DATABASE_HOST".to_string(), "host.docker.internal".to_string()),
                ("DATABASE_USER".to_string(), db_user.to_string()),
                ("DATABASE_PASSWORD".to_string(), db_password.to_string()),
                ("DATABASE_NAME".to_string(), db_name.to_string()),
                ("BLOCKCHAIN_RPC_URL".to_string(), format!("http://host.docker.internal:{}", rpc_port)),
                ("DATA_FETCHER_URL".to_string(), data_fetcher_url),
                ("BATCHES_PROCESSING_POLLING_INTERVAL".to_string(), "1000".to_string()),
            ])),
            extra_hosts: Some(vec!["host.docker.internal:host-gateway".to_string()]),
        }
    }

    pub fn get_config_path(ecosystem_base_path: &Path, chain_name: &str) -> PathBuf {
        ecosystem_base_path
            .join(LOCAL_CHAINS_PATH)
            .join(chain_name)
            .join(LOCAL_CONFIGS_PATH)
            .join(LOCAL_APPS_PATH)
            .join(EXPLORER_BACKEND_DOCKER_COMPOSE_FILE)
    }
}
