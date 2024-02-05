mod api;
mod archives;
mod cache;
mod config;
mod messages;
mod nxm_listener;
mod ui;
mod util;

use std::env::args;
use std::error::Error;

use api::{Client, Downloads};
use archives::Archives;
use cache::Cache;
use config::{Config, ConfigBuilder};
use messages::Messages;

/* dmodman acts as an url handler for nxm:// links in order for the "download with mod manager" button to work on
 * NexusMods.
 * If the program is invoked without argument, it starts the TUI unless another instance is already running.
 * If an nxm:// link is passed as an argument, we try to queue it in an already running instance. If none exists, we
 * start the TUI normally and queue the download.
 */

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut nxm_str_opt: Option<&str> = None;
    let mut is_interactive = true;

    let args: Vec<String> = args().collect();
    if args.len() > 2 {
        println!("Too many arguments. Invoke dmodman without arguments or with an nxm:// URL.");
        return Ok(());
    } else if let Some(first_arg) = args.get(1) {
        if first_arg.starts_with("nxm://") {
            nxm_str_opt = Some(first_arg);
        } else if first_arg == "-d" {
            is_interactive = false;
        } else {
            println!("Arguments are expected only when acting as an nxm:// URL handler.");
            return Ok(());
        }
    }

    // If dmodman is already running, we queue any possible nxm:// URL, then exit early.
    let nxm_rx = match nxm_listener::queue_download_else_bind_to_socket(nxm_str_opt).await? {
        Some(v) => v,
        None => return Ok(()),
    };

    let msgs = Messages::new(is_interactive);

    // TODO config is cloned needlessly in a few places
    let mut config = match ConfigBuilder::load() {
        Ok(cb) => cb,
        Err(_) => ConfigBuilder::default(),
    }
    .build()?;
    if config.apikey.is_none() {
        if let Some(apikey) = ui::sso::start_apikey_flow().await {
            config.apikey = Some(apikey);
            config.save_apikey()?;
        } else {
            // This program doesn't really do anything without an API key, but continue anyway.
            msgs.push("No API key configured. API connections are disabled.").await;
        }
    }

    let cache = Cache::new(&config).await?;
    let client = Client::new(&config).await;
    let downloads = Downloads::new(&cache, &client, &config, &msgs).await;
    downloads.resume_on_startup().await;

    if let Some(nxm_str) = nxm_str_opt {
        let _ = downloads.queue(nxm_str.to_string()).await;
    }

    if is_interactive {
        {
            let downloads = downloads.clone();
            let msgs = msgs.clone();
            tokio::task::spawn(async move {
                nxm_listener::listen_for_downloads(downloads, msgs, nxm_rx).await;
            });
        }

        let archive = Archives::new(config.clone(), msgs.clone());
        ui::MainUI::new(cache, client, config, downloads, msgs, archive).run().await;
    } else {
        nxm_listener::listen_for_downloads(downloads, msgs, nxm_rx).await;
    }

    Ok(())
}