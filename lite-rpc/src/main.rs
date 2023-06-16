use std::time::Duration;

use clap::Parser;
use dotenv::dotenv;
use lite_rpc::{bridge::LiteBridge, cli::Args};
use log::info;
use prometheus::{opts, register_int_counter, IntCounter};
use solana_sdk::signature::Keypair;
use std::env;

async fn get_identity_keypair(identity_from_cli: &String) -> Keypair {
    if let Ok(identity_env_var) = env::var("IDENTITY") {
        if let Ok(identity_bytes) = serde_json::from_str::<Vec<u8>>(identity_env_var.as_str()) {
            Keypair::from_bytes(identity_bytes.as_slice()).unwrap()
        } else {
            // must be a file
            let identity_file = tokio::fs::read_to_string(identity_env_var.as_str())
                .await
                .expect("Cannot find the identity file provided");
            let identity_bytes: Vec<u8> = serde_json::from_str(&identity_file).unwrap();
            Keypair::from_bytes(identity_bytes.as_slice()).unwrap()
        }
    } else if identity_from_cli.is_empty() {
        Keypair::new()
    } else {
        let identity_file = tokio::fs::read_to_string(identity_from_cli.as_str())
            .await
            .expect("Cannot find the identity file provided");
        let identity_bytes: Vec<u8> = serde_json::from_str(&identity_file).unwrap();
        Keypair::from_bytes(identity_bytes.as_slice()).unwrap()
    }
}

lazy_static::lazy_static! {
    static ref RESTARTS: IntCounter =
    register_int_counter!(opts!("literpc_rpc_restarts", "Nutber of times lite rpc restarted")).unwrap();
}

#[tokio::main(flavor = "multi_thread", worker_threads = 16)]
pub async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let Args {
        rpc_addr,
        ws_addr,
        lite_rpc_ws_addr,
        lite_rpc_http_addr,
        clean_interval_ms,
        fanout_size,
        enable_postgres,
        prometheus_addr,
        identity_keypair,
        maximum_retries_per_tx,
        transaction_retry_after_secs,
    } = Args::parse();

    dotenv().ok();

    let clean_interval_ms = Duration::from_millis(clean_interval_ms);

    let enable_postgres = enable_postgres
        || if let Ok(enable_postgres_env_var) = env::var("PG_ENABLED") {
            enable_postgres_env_var != "false"
        } else {
            false
        };

    let retry_after = Duration::from_secs(transaction_retry_after_secs);

    loop {
        let identity = get_identity_keypair(&identity_keypair).await;

        let services = LiteBridge::new(
            rpc_addr.clone(),
            ws_addr.clone(),
            fanout_size,
            identity,
            retry_after,
            maximum_retries_per_tx,
        )
        .await?
        .start_services(
            lite_rpc_http_addr.clone(),
            lite_rpc_ws_addr.clone(),
            clean_interval_ms,
            enable_postgres,
            prometheus_addr.clone(),
        );

        let ctrl_c_signal = tokio::signal::ctrl_c();

        tokio::select! {
            res = services => {
                const RESTART_DURATION: Duration = Duration::from_secs(20);

                log::error!("Services quit unexpectedly {res:?} restarting in {RESTART_DURATION:?}");
                tokio::time::sleep(RESTART_DURATION).await;
                log::error!("Restarting services");
                RESTARTS.inc();
            }
            _ = ctrl_c_signal => {
                info!("Received ctrl+c signal");

                break Ok(())
            }
        }
    }
}
