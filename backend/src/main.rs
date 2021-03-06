#[macro_use]
extern crate actix_web;

#[cfg(feature = "ui")]
use rsbt_frontend::*;

use actix::prelude::*;
use actix_identity::{CookieIdentityPolicy, Identity, IdentityService};
use actix_multipart::Multipart;
use actix_web::{
    dev::Payload, error::ErrorUnauthorized, http, middleware, web, App, Error, FromRequest,
    HttpRequest, HttpResponse, HttpServer, Responder,
};
use bytes::Bytes;
use dotenv::dotenv;
use exitfailure::ExitFailure;
use futures::{future::abortable, stream::select_all, StreamExt};
use log::{debug, error, info, trace};
use openid::{DiscoveredClient, Options, Token, Userinfo};
use reqwest;
use rsbt_service::{
    app::{events::TorrentEvent, *},
    types::*,
    RsbtError,
};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::HashMap,
    path::{Path, PathBuf},
    pin::Pin,
    task::{Context, Poll},
};
use structopt::StructOpt;
use tokio::{
    fs,
    sync::{
        mpsc::{self, Receiver, Sender},
        RwLock,
    },
    time::{delay_for, Duration},
};
use url::Url;

mod cli;
mod event_stream;
mod login;
mod model;
#[cfg(feature = "sandbox")]
mod sandbox;
mod session;
mod torrents;
mod uploads;

use event_stream::*;
use login::*;
#[cfg(feature = "sandbox")]
use sandbox::*;
use session::*;
use torrents::*;
use uploads::*;

lazy_static::lazy_static! {
static ref RSBT_UI_HOST: String = std::env::var("RSBT_UI_HOST").unwrap_or_else(|_| "http://localhost:8080".to_string());
static ref RSBT_BIND: String = std::env::var("RSBT_BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
static ref RSBT_OPENID_CLIENT_ID: String = std::env::var("RSBT_OPENID_CLIENT_ID").unwrap_or_else(|_| "web_app".to_string());
static ref RSBT_OPENID_CLIENT_SECRET: String = std::env::var("RSBT_OPENID_CLIENT_SECRET").unwrap_or_else(|_| "web_app".to_string());
static ref RSBT_OPENID_ISSUER: String = std::env::var("RSBT_OPENID_ISSUER").unwrap_or_else(|_| "http://keycloak:9080/auth/realms/jhipster".to_string());
static ref RSBT_ALLOW: String = std::env::var("RSBT_ALLOW").unwrap_or_else(|_| "user@localhost".to_string());
}

#[derive(Serialize, Deserialize, Debug)]
struct Failure {
    error: String,
}

fn host(path: &str) -> String {
    RSBT_UI_HOST.clone() + path
}

async fn load_settings<P: AsRef<Path>>(config_path: P) -> Result<Settings, std::io::Error> {
    let config_file = config_path.as_ref().join("rsbt.toml");
    if !config_file.is_file() {
        return Ok(Settings::default());
    }
    let config_file = fs::read_to_string(config_file).await?;
    Ok(toml::from_str(&config_file)?)
}

#[actix_rt::main]
async fn main() -> Result<(), ExitFailure> {
    dotenv().ok();

    let cli = cli::Cli::from_args();

    env_logger::init();

    let config_path = cli
        .config_path
        .map(PathBuf::from)
        .unwrap_or_else(rsbt_service::default_app_dir);

    let settings = load_settings(&config_path).await?.override_with(cli.config);

    let local = cli.local;

    let client = if !local {
        let client = connect_to_openid_provider().await?;
        Some(web::Data::new(client))
    } else {
        None
    };

    debug!("starting torrents process with settings: {:?}", settings);
    let properties = (settings, config_path).into();
    debug!("properties: {:?}", properties);

    let sessions = web::Data::new(Sessions::new(&properties, local).await?);

    let storage_path = properties.storage.clone();

    let rsbt_app = RsbtApp::new(properties);

    let current_torrents = rsbt_app.init_storage().await?;

    let (broadcaster, mut broadcaster_sender) = init_broadcaster();

    let mut download_events_sender = init_rsbt_app(rsbt_app);

    for torrent in current_torrents.torrents {
        let torrent_path = storage_path.join(&torrent.file);

        if torrent_path.exists() {
            let data = fs::read(&torrent_path).await?;

            let (request_response, receiver) = RequestResponse::new(RsbtCommandAddTorrent {
                data,
                filename: torrent.file,
                state: torrent.state,
            });

            download_events_sender
                .send(RsbtCommand::AddTorrent(request_response))
                .await
                .map_err(RsbtError::from)?;

            let torrent_download = receiver.await??;

            if let Err(err) = broadcaster_sender
                .send(BroadcasterMessage::Subscribe(torrent_download))
                .await
            {
                error!("cannot send subscribe message: {}", err);
            }
        }
    }

    let sender = web::Data::new(download_events_sender);
    let broadcaster_sender = web::Data::new(broadcaster_sender);

    HttpServer::new(move || {
        let mut app = App::new()
            .wrap(middleware::Logger::default())
            .wrap(IdentityService::new(
                CookieIdentityPolicy::new(&[0; 32])
                    .name("auth-rsbt")
                    .secure(false),
            ))
            .app_data(broadcaster.clone())
            .app_data(sessions.clone())
            .app_data(sender.clone())
            .app_data(broadcaster_sender.clone())
            .service(authorize)
            .service(login_get)
            .service(
                web::scope("/api")
                    .service(torrent_list)
                    .service(torrent_detail)
                    .service(torrent_delete)
                    .service(torrent_create_action)
                    .service(torrent_peer_list)
                    .service(torrent_announce_list)
                    .service(torrent_file_list)
                    .service(torrent_piece_list)
                    .service(torrent_file_download)
                    .service(upload)
                    .service(account)
                    .service(logout)
                    .service(stream),
            );
        if let Some(client) = &client {
            app = app.app_data(client.clone());
        }
        #[cfg(feature = "sandbox")]
        {
            debug!("adding sandbox at /sandbox...");
            app = app.service(
                web::scope("/sandbox")
                    .service(sandbox)
                    .service(rsbt_javascript_module)
                    .service(rsbt_css)
                    .service(upload_form)
                    .service(stream_page)
                    .service(torrent_piece_page),
            );
        }
        #[cfg(feature = "ui")]
        {
            debug!("serving frontend files...");
            let generated_files = generate_files();
            app = app.service(actix_web_static_files::ResourceFiles::new(
                "/",
                generated_files,
            ));
        }
        app
    })
    .bind(RSBT_BIND.as_str())?
    .run()
    .await?;

    Ok(())
}

fn init_rsbt_app(rsbt_app: RsbtApp) -> Sender<RsbtCommand> {
    let (download_events_sender, download_events_receiver) =
        mpsc::channel(rsbt_service::DEFAULT_CHANNEL_BUFFER);

    let download_events_task_sender = download_events_sender.clone();

    let rsbt_app_task = async move {
        if let Err(err) = rsbt_app
            .processing_loop(download_events_task_sender, download_events_receiver)
            .await
        {
            error!("problem detected: {}", err);
        }
    };
    Arbiter::spawn(rsbt_app_task);

    download_events_sender
}
enum BroadcasterMessage {
    Send(TorrentEvent),
    Subscribe(TorrentDownload),
    Unsubscribe(usize),
}

fn init_broadcaster() -> (web::Data<Broadcaster>, Sender<BroadcasterMessage>) {
    let broadcaster = web::Data::new(Broadcaster::new());
    let broadcaster_timer = broadcaster.clone();

    let (broadcaster_sender, mut broadcaster_receiver) =
        mpsc::channel(rsbt_service::DEFAULT_CHANNEL_BUFFER);

    let task_broadcaster_sender = broadcaster_sender.clone();
    let task = async move {
        let mut subscriptions = HashMap::new();
        while let Some(message) = broadcaster_receiver.next().await {
            match message {
                BroadcasterMessage::Send(torrent_event) => {
                    let json_message = serde_json::to_string(&torrent_event).unwrap();
                    debug!("sending broadcast: {}", json_message);
                    if let Err(ok_clients) = broadcaster_timer.message(&json_message).await {
                        debug!("refresh client list");
                        *broadcaster_timer.clients.write().await = ok_clients;
                    }
                }
                BroadcasterMessage::Subscribe(torrent_download) => {
                    let id = torrent_download.id;
                    let mut task_broadcaster_sender = task_broadcaster_sender.clone();
                    let (subscription_task, subscription_abort_handle) = abortable(async move {
                        let mut messages = select_all(vec![
                            torrent_download
                                .storage_state_watch
                                .map(move |x| TorrentEvent::Storage {
                                    id,
                                    read: x.bytes_read,
                                    write: x.bytes_write,
                                    left: x.pieces_left,
                                })
                                .boxed(),
                            torrent_download
                                .statistics_watch
                                .map(move |x| TorrentEvent::Stat {
                                    id,
                                    tx: x.uploaded,
                                    rx: x.downloaded,
                                })
                                .boxed(),
                        ]);
                        while let Some(message) = messages.next().await {
                            if let Err(err) = task_broadcaster_sender
                                .send(BroadcasterMessage::Send(message))
                                .await
                            {
                                error!("cannot send from subscription: {}", err)
                            }
                            delay_for(Duration::from_millis(500)).await;
                        }
                    });
                    tokio::spawn(subscription_task);
                    subscriptions.insert(id, subscription_abort_handle);
                }
                BroadcasterMessage::Unsubscribe(id) => {
                    if let Some(abort_handle) = subscriptions.remove(&id) {
                        abort_handle.abort();
                    }
                }
            }
        }
    };
    Arbiter::spawn(task);
    (broadcaster, broadcaster_sender)
}
