use clap::Parser;
use common::{db::DatabaseConfig, Prompt};
use config::ChainConfig;
use serde::{Deserialize, Serialize};
use slugify_rs::slugify;
use url::Url;

#[derive(Debug, Serialize, Deserialize, Parser)]
pub struct ExplorerArgs {
    #[clap(
        long,
        default_value = "3010",
        help = "The port number for the block explorer app"
    )]
    pub port: u16,
}
