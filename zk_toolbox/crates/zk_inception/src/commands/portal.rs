use std::{collections::HashMap, path::Path, str::FromStr};

use anyhow::{anyhow, Context};
use common::{docker, ethereum, logger};
use config::{
    portal::*, traits::{ReadConfig, SaveConfig}, AppsEcosystemConfig, ChainConfig, EcosystemConfig, PortalAppChainConfig
};
use ethers::types::Address;
use types::{BaseToken, TokenInfo};
use xshell::Shell;
use url::Url;

use crate::{
    commands::args::PortalArgs,
    consts::{L2_BASE_TOKEN_ADDRESS, PORTAL_DOCKER_CONTAINER_PORT, PORTAL_DOCKER_IMAGE},
    messages::{
        msg_portal_starting_on, MSG_PORTAL_FAILED_TO_CREATE_ANY_CHAIN_CONFIG_ERR,
        MSG_PORTAL_FAILED_TO_CREATE_CONFIG_ERR, MSG_PORTAL_FAILED_TO_RUN_DOCKER_ERR,
    },
};

async fn create_hyperchain_config(chain_config: &ChainConfig) -> anyhow::Result<PortalChainConfig> {
    // Get L2 RPC URL from general config
    let general_config = chain_config.get_general_config()?;
    let rpc_url: &String = general_config
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
    // Build L1 network config
    let l1_network = Some(L1NetworkConfig {
        id: chain_config.l1_network.chain_id(),
        name: chain_config.l1_network.to_string(),
        network: chain_config.l1_network.to_string().to_lowercase(),
        native_currency: TokenInfo::eth(),
        rpc_urls: RpcUrls {
            default: RpcUrlConfig {
                http: vec![l1_rpc_url.to_string()],
            },
            public: RpcUrlConfig {
                http: vec![l1_rpc_url.to_string()],
            },
        },
    });
    // Base token:
    let (base_token_addr, base_token_info) = if chain_config.base_token == BaseToken::eth() {
        (format!("{:?}", Address::zero()), TokenInfo::eth())
    } else {
        (
            format!("{:?}", chain_config.base_token.address),
            ethereum::get_token_info(chain_config.base_token.address, l1_rpc_url.to_string())
                .await?,
        )
    };
    let tokens = vec![TokenConfig {
        address: L2_BASE_TOKEN_ADDRESS.to_string(),
        l1_address: Some(base_token_addr.to_string()),
        symbol: base_token_info.symbol,
        decimals: base_token_info.decimals,
        name: Some(base_token_info.name.to_string()),
    }];
    // Build hyperchain config
    Ok(PortalChainConfig {
        network: NetworkConfig {
            id: chain_config.chain_id.as_u64(),
            key: chain_config.name.clone(),
            name: chain_config.name.clone(),
            rpc_url: rpc_url.to_string(),
            l1_network,
            public_l1_network_id: None,
            block_explorer_url: None,
            block_explorer_api: None,
            hidden: None,
        },
        tokens,
    })
}

pub async fn create_portal_config(
    ecosystem_config: &EcosystemConfig,
) -> anyhow::Result<PortalRuntimeConfig> {
    let chains: Vec<String> = ecosystem_config.list_of_chains();
    let mut hyperchains_config = Vec::new();
    for chain in chains {
        if let Some(chain_config) = ecosystem_config.load_chain(Some(chain.clone())) {
            if let Ok(hyperchain_config) = create_hyperchain_config(&chain_config).await {
                hyperchains_config.push(hyperchain_config)
            }
        }
    }
    if hyperchains_config.is_empty() {
        anyhow::bail!("Failed to create any valid hyperchain config")
    }
    let runtime_config = PortalRuntimeConfig {
        node_type: "hyperchain".to_string(),
        hyperchains_config,
    };
    Ok(runtime_config)
}

pub async fn create_and_save_portal_chain_config(
    chain_config: &ChainConfig,
    shell: &Shell,
) -> anyhow::Result<PortalChainConfig> {
    let portal_config = create_hyperchain_config(chain_config).await?;
    let config_path = PortalChainConfig::get_config_path(&shell.current_dir(), &chain_config.name);
    portal_config.save(shell, config_path)?;
    Ok(portal_config)
}

pub async fn create_and_save_portal_config(
    ecosystem_config: &EcosystemConfig,
    shell: &Shell,
) -> anyhow::Result<PortalRuntimeConfig> {
    let portal_config = create_portal_config(ecosystem_config).await?;
    let config_path = PortalRuntimeConfig::get_config_path(&shell.current_dir());
    portal_config.save(shell, config_path)?;
    Ok(portal_config)
}

pub async fn build_portal_app_config(
    ecosystem_config: &EcosystemConfig,
    chain_config: &ChainConfig,
    shell: &Shell,
) -> anyhow::Result<PortalAppChainConfig> {
    let general_config = chain_config.get_general_config()?;
    let rpc_url = general_config
        .api_config
        .as_ref()
        .map(|api_config| &api_config.web3_json_rpc.http_url)
        .context("api_config")?;

    let hyperchain_config = create_hyperchain_config(chain_config).await?;
    let l2_rpc_url = Url::from_str(&hyperchain_config.network.rpc_url)?;
    let portal_app_config = PortalAppChainConfig {
        l2_rpc_url,
    };
    Ok(portal_app_config)
}

async fn generate_portal_runtime_config(
    shell: &Shell,
    ecosystem_config: &EcosystemConfig,
    chain_names: Vec<String>,
) -> anyhow::Result<PortalRuntimeConfig> {
    let ecosystem_base_path = &shell.current_dir();
    let mut hyperchains_config = Vec::new();

    for chain_name in chain_names {
        let config_path = PortalChainConfig::get_config_path(ecosystem_base_path, &chain_name);
        
        let portal_chain_config = match PortalChainConfig::read(shell, &config_path) {
            Ok(config) => Some(config),
            Err(_) => {
                match ecosystem_config.load_chain(Some(chain_name.clone())) {
                    Some(chain_config) => {
                        match create_hyperchain_config(&chain_config).await {
                            Ok(config) => Some(config),
                            Err(_) => None,
                        }
                    },
                    None => None,
                }
            },
        };
        if let Some(config) = portal_chain_config {
            hyperchains_config.push(config);
        }
    }
    if hyperchains_config.is_empty() {
        anyhow::bail!(MSG_PORTAL_FAILED_TO_CREATE_ANY_CHAIN_CONFIG_ERR)
    }
    let runtime_config = PortalRuntimeConfig::new(hyperchains_config);
    Ok(runtime_config)
}


pub async fn run(shell: &Shell, args: PortalArgs) -> anyhow::Result<()> {
    let ecosystem_config: EcosystemConfig = EcosystemConfig::from_file(shell)?;
    // Get ecosystem level apps.yaml config
    let apps_config = AppsEcosystemConfig::read_or_create_default(shell)?;
    // What chains to run the portal for?
    let chains_enabled = apps_config.portal.chains_enabled
        .unwrap_or_else(|| ecosystem_config.list_of_chains());
    
    // Generate portal runtime config
    let runtime_config = generate_portal_runtime_config(shell, &ecosystem_config, chains_enabled)
        .await
        .context(MSG_PORTAL_FAILED_TO_CREATE_CONFIG_ERR)?;
    let config_path = PortalRuntimeConfig::get_config_path(&shell.current_dir());
    runtime_config.save(shell, &config_path)?;

    logger::info(format!(
        "Using portal config file at {}",
        config_path.display()
    ));

    logger::info(msg_portal_starting_on("127.0.0.1", args.port));
    run_portal(shell, &config_path, args.port)?;
    Ok(())
}

fn run_portal(shell: &Shell, config_file_path: &Path, port: u16) -> anyhow::Result<()> {
    let port_mapping = format!("{}:{}", port, PORTAL_DOCKER_CONTAINER_PORT);
    let volume_mapping = format!("{}:/usr/src/app/dist/config.js", config_file_path.display());

    let mut docker_args: HashMap<String, String> = HashMap::new();
    docker_args.insert("--platform".to_string(), "linux/amd64".to_string());
    docker_args.insert("-p".to_string(), port_mapping);
    docker_args.insert("-v".to_string(), volume_mapping);

    docker::run(shell, PORTAL_DOCKER_IMAGE, docker_args)
        .with_context(|| MSG_PORTAL_FAILED_TO_RUN_DOCKER_ERR)?;
    Ok(())
}
