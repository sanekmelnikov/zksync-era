use std::{collections::{HashMap, HashSet}, path::{Path, PathBuf}, str::FromStr};

use anyhow::{anyhow, Context};
use common::{db, docker, logger, Prompt};
use config::{
    docker_compose::*, explorer::*, explorer_compose::*, traits::{ReadConfig, SaveConfig}, ApiConfig, AppsChainConfig, AppsEcosystemConfig, ChainConfig, DataFetcherConfig, EcosystemConfig, ExplorerAppChainConfig, ServicesConfig, WorkerConfig
};
use slugify_rs::slugify;
use types::{BaseToken, TokenInfo};
use url::Url;
use xshell::Shell;

use crate::{
    commands::args::ExplorerArgs,
    consts::L2_BASE_TOKEN_ADDRESS,
    defaults::{generate_explorer_db_name, DATABASE_EXPLORER_URL, DEFAULT_EXPLORER_API_PORT, DEFAULT_EXPLORER_DATA_FETCHER_PORT, DEFAULT_EXPLORER_WORKER_PORT},
    messages::{
        msg_explorer_db_name_prompt, msg_explorer_db_url_prompt, msg_explorer_initializing_database_for, msg_portal_starting_on, MSG_EXPLORER_FAILED_TO_DROP_DATABASE_ERR, MSG_PORTAL_FAILED_TO_CREATE_ANY_CHAIN_CONFIG_ERR, MSG_PORTAL_FAILED_TO_CREATE_CONFIG_ERR, MSG_PORTAL_FAILED_TO_RUN_DOCKER_ERR
    },
    utils::ports::EcosystemPortsScanner,
};

async fn create_explorer_chain_config(chain_config: &ChainConfig) -> anyhow::Result<ExplorerChainConfig> {
    // Get L2 RPC URL from general config
    let general_config = chain_config.get_general_config()?;
    let rpc_url = general_config
        .api_config
        .as_ref()
        .map(|api_config| &api_config.web3_json_rpc.http_url)
        .context("api_config")?;
    // Get L1 RPC URL from secrects config
    let secrets_config = chain_config.get_secrets_config()?;
    let l1_rpc_url = secrets_config
        .l1
        .as_ref()
        .map(|l1| l1.l1_rpc_url.expose_str())
        .context("l1")?;
    // Build network config
    Ok(ExplorerChainConfig {
        name: chain_config.name.clone(),
        l2_network_name: chain_config.name.clone(),
        l2_chain_id: chain_config.chain_id.as_u64(),
        rpc_url: rpc_url.to_string(),
        api_url: "http://127.0.0.1:3020".to_string(), // TODO: implement
        base_token_address: L2_BASE_TOKEN_ADDRESS.to_string(),
        hostnames: Vec::new(),  // TODO: implement
        icon: "/images/icons/zksync-arrows.svg".to_string(),
        maintenance: false,
        published: true,
        bridge_url: None, // TODO: implement
        l1_explorer_url: None, // TODO: implement
        verification_api_url: Some("http://localhost:3070".to_string()),
    })
}

pub async fn create_explorer_config(
    ecosystem_config: &EcosystemConfig
) -> anyhow::Result<ExplorerRuntimeConfig> {
    let chains: Vec<String> = ecosystem_config.list_of_chains();

    let mut environment_config = Vec::new();
    for chain in chains {
        if let Some(chain_config) = ecosystem_config.load_chain(Some(chain.clone())) {
            if let Ok(network_config) = create_explorer_chain_config(&chain_config).await {
                environment_config.push(network_config)
            }
        }
    }
    if environment_config.is_empty() {
        anyhow::bail!("Failed to create any valid network config")
    }
    let runtime_config = ExplorerRuntimeConfig {
        app_environment: "default".to_string(),
        environment_config,
    };
    Ok(runtime_config)
}

pub async fn create_and_save_explorer_chain_config(
    chain_config: &ChainConfig,
    shell: &Shell,
) -> anyhow::Result<ExplorerChainConfig> {
    let explorer_config = create_explorer_chain_config(chain_config).await?;
    let config_path = ExplorerChainConfig::get_config_path(&shell.current_dir(), &chain_config.name);
    explorer_config.save(shell, config_path)?;
    Ok(explorer_config)
}

pub async fn create_and_save_explorer_config(
    ecosystem_config: &EcosystemConfig,
    shell: &Shell,
) -> anyhow::Result<ExplorerRuntimeConfig> {
    let explorer_config = create_explorer_config(ecosystem_config).await?;
    let config_path = ExplorerRuntimeConfig::get_config_path(&shell.current_dir());
    explorer_config.save(shell, config_path)?;
    Ok(explorer_config)
}

pub async fn build_explorer_app_config(
    ecosystem_config: &EcosystemConfig,
    chain_config: &ChainConfig,
    shell: &Shell,
) -> anyhow::Result<ExplorerAppChainConfig> {
    let network_config = create_explorer_chain_config(chain_config).await?;
    let services_config = build_explorer_services_app_config(
        ecosystem_config, chain_config, shell
    )?;
    let verification_api_url = match &network_config.verification_api_url {
        Some(url) => Some(Url::parse(url)?),
        None => None,
    };
    let l2_rpc_url = Url::parse(&network_config.rpc_url)?;
    let explorer_app_config = ExplorerAppChainConfig {
        l2_rpc_url,
        verification_api_url,
        database_url: None,
        services: Some(services_config),
    };
    Ok(explorer_app_config)
}

pub fn build_explorer_services_app_config(
    ecosystem_config: &EcosystemConfig,
    chain_config: &ChainConfig,
    shell: &Shell,
) -> anyhow::Result<ServicesConfig> {
    // call allocate_explorer_services_ports here
    let api_http_url = Url::parse(format!("http://127.0.0.1:{}", 3002).as_str())?;
    let services_config = ServicesConfig {
        api: ApiConfig {
            http_url: api_http_url,
            http_port: 3002,
            metrics_port: 3005,
        },
        data_fetcher: DataFetcherConfig {
            http_port: 3040,
        },
        worker: WorkerConfig {
            http_port: 3001,
            batches_processing_polling_interval: 1000,
        }
    };
    Ok(services_config)
}

pub async fn run(shell: &Shell, args: ExplorerArgs) -> anyhow::Result<()> {
    let ecosystem_config: EcosystemConfig = EcosystemConfig::from_file(shell)?;
    let ecosystem_path = shell.current_dir();
    // Get ecosystem level apps.yaml config
    let apps_config = AppsEcosystemConfig::read_or_create_default(shell)?;
    // What chains to run the explorer for
    let chains_enabled = apps_config.explorer.chains_enabled
        .unwrap_or_else(|| ecosystem_config.list_of_chains());
    
    //  Keep track of allocated ports (initialized lazily)
    let mut allocated_ports: HashSet<u16> = HashSet::new();
    
    // ========== EXPLORER DOCKER COMPOSE ==========
    // Initialize chains if needed
    let mut explorer_chain_configs = Vec::new();
    let mut backend_configs = Vec::new();
    for chain_name in chains_enabled.iter() {
        let chain_config = ecosystem_config.load_chain(Some(chain_name.clone()))
            .ok_or_else(|| anyhow::anyhow!("Failed to load chain config for {}", chain_name))?;

        // Should initialize? Check if explorer backend docker compose exists
        let mut should_initialize = false;
        let backend_compose_path = ExplorerBackendComposeConfig::get_config_path(&ecosystem_path, chain_name);
        let backend_compose_config = match ExplorerBackendComposeConfig::read(shell, &backend_compose_path) {
            Ok(config) => config,
            Err(_) => {
                should_initialize = true;
                // Initialize the backend if it doesn't exist
                logger::info(msg_explorer_initializing_database_for(&chain_name));
                let db_config = fill_database_values_with_prompt(&chain_config);
                initialize_explorer_database(&db_config).await?;

                // Allocate ports for backend services
                let service_ports = allocate_explorer_services_ports(&ecosystem_path, &mut allocated_ports)?;
                let backend_service_config = ExplorerBackendServiceConfig {
                    db_url: db_config.full_url().to_string(),
                    rpc_port: 3050,
                    service_ports,
                };

                // Create and save ExplorerBackendComposeConfig
                let backend_compose_config = ExplorerBackendComposeConfig::new(chain_name, backend_service_config)?;
                backend_compose_config.save(shell, &backend_compose_path)?;
                backend_compose_config
            }
        };
        backend_configs.push(backend_compose_config);
        
        let explorer_chain_config_path = ExplorerChainConfig::get_config_path(&ecosystem_path, chain_name);
        let explorer_chain_config = match should_initialize {
            true => create_and_save_explorer_chain_config(&chain_config, shell).await?,
            false => {
                match ExplorerChainConfig::read(shell, &explorer_chain_config_path) {
                    Ok(config) => config,
                    Err(_) => create_and_save_explorer_chain_config(&chain_config, shell).await?
                }
            }
        };
        explorer_chain_configs.push(explorer_chain_config);
    }

    // Generate explorer runtime config
    let explorer_runtime_config = ExplorerRuntimeConfig::new(explorer_chain_configs);
    let explorer_runtime_config_path = ExplorerRuntimeConfig::get_config_path(&ecosystem_path);
    explorer_runtime_config.save(shell, &explorer_runtime_config_path)?;

    // Generate explorer docker compose
    let app_config = ExplorerAppServiceConfig {
        port: apps_config.explorer.http_port,
        config_path: explorer_runtime_config_path,
    };
    let explorer_compose_config = ExplorerComposeConfig::new(app_config, backend_configs)?;
    let explorer_compose_path = ExplorerComposeConfig::get_config_path(&ecosystem_path);
    explorer_compose_config.save(&shell, &explorer_compose_path)?;

    // ========== RUNNING ==========
    logger::info(format!(
        "Using generated docker compose file at {}",
        explorer_compose_path.display()
    ));
    logger::info(format!("Starting explorer app at 127.0.0.1:{}", args.port));
    run_explorer(shell, &explorer_compose_path)?;
    Ok(())
}

fn run_explorer(shell: &Shell, explorer_compose_config_path: &Path) -> anyhow::Result<()> {
    if let Some(docker_compose_file) = explorer_compose_config_path.to_str() {
        docker::up_and_running(shell, docker_compose_file)
            .with_context(|| "Failed to run docker compose for Explorer")?;
    } else {
        anyhow::bail!("Invalid docker compose file");
    }
    Ok(())
}

fn fill_database_values_with_prompt(config: &ChainConfig) -> db::DatabaseConfig {
    let defaul_db_name: String = generate_explorer_db_name(config);
    let chain_name = config.name.clone();
    let explorer_db_url = Prompt::new(&msg_explorer_db_url_prompt(&chain_name))
        .default(DATABASE_EXPLORER_URL.as_str())
        .ask();
    let explorer_db_name: String = Prompt::new(&msg_explorer_db_name_prompt(&chain_name))
        .default(&defaul_db_name)
        .ask();
    let explorer_db_name = slugify!(&explorer_db_name, separator = "_");
    return db::DatabaseConfig::new(explorer_db_url, explorer_db_name);
}

pub async fn initialize_explorer_database(
    explorer_db_config: &db::DatabaseConfig,
) -> anyhow::Result<()> {
    db::drop_db_if_exists(explorer_db_config)
        .await
        .context(MSG_EXPLORER_FAILED_TO_DROP_DATABASE_ERR)?;
    db::init_db(explorer_db_config).await?;
    Ok(())
}

pub fn allocate_explorer_services_ports(
    ecosystem_path: &Path,
    allocated_ports: &mut HashSet<u16>,
) -> anyhow::Result<ExplorerBackendServicePorts> {
    if allocated_ports.is_empty() {
        let ports = EcosystemPortsScanner::scan(ecosystem_path)?;
        allocated_ports.extend(ports.get_assigned_ports());
    }

    let mut service_ports = ExplorerBackendServicePorts {
        api_port: DEFAULT_EXPLORER_API_PORT,
        data_fetcher_port: DEFAULT_EXPLORER_DATA_FETCHER_PORT,
        worker_port: DEFAULT_EXPLORER_WORKER_PORT,
    };

    let offset = 100;
    while allocated_ports.contains(&service_ports.api_port) ||
          allocated_ports.contains(&service_ports.data_fetcher_port) ||
          allocated_ports.contains(&service_ports.worker_port) {
        service_ports.api_port += offset;
        service_ports.data_fetcher_port += offset;
        service_ports.worker_port += offset;
    }
    allocated_ports.insert(service_ports.api_port);
    allocated_ports.insert(service_ports.data_fetcher_port);
    allocated_ports.insert(service_ports.worker_port);
    Ok(service_ports)
}
