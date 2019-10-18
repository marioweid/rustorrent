#[macro_use]
extern crate actix_web;

#[macro_use]
extern crate serde_json;

use actix::prelude::*;
use actix_web::error::ErrorInternalServerError;

use actix_web::{web, App, Error, HttpResponse, HttpServer, Responder};
use actix_web_static_files;
use bytes::Bytes;
use exitfailure::ExitFailure;
use failure::ResultExt;
use futures::{Async, Poll, Stream};
use rand::prelude::*;
use rustorrent_web_resources::*;
use serde::{Deserialize, Serialize};
use std::io;
use std::sync::RwLock;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::timer::{Delay, Interval};

#[derive(Serialize, Deserialize, Clone)]
struct TorrentInfo {
    name: String,
    total: usize,
    downloaded: usize,
    uploaded: usize,
    active: bool,
}

struct AppState {
    torrents: Vec<TorrentInfo>,
}

const INDEX: &str = include_str!("../static/templates/index.html");

#[get("/")]
fn index() -> impl Responder {
    HttpResponse::Ok()
        .header("content-type", "text/html")
        .body(INDEX)
}

#[get("/torrents")]
fn torrent_list(app_state: web::Data<AppState>) -> impl Responder {
    web::Json(app_state.torrents.clone())
}

#[get("/stream")]
fn stream(
    app_state: web::Data<AppState>,
    broadcaster: web::Data<RwLock<Broadcaster>>,
) -> impl Responder {
    let rx = broadcaster.write().unwrap().new_client();
    HttpResponse::Ok()
        .content_type("text/event-stream")
        .keep_alive()
        .streaming(rx)
}

fn main() -> Result<(), ExitFailure> {
    let system = System::new(env!("CARGO_PKG_NAME"));
    let app_state = web::Data::new(AppState {
        torrents: vec![TorrentInfo {
            name: "ferris2.gif".into(),
            total: 308_189,
            downloaded: 100_100,
            uploaded: 55_020,
            active: true,
        }],
    });

    let broadcaster = web::Data::new(RwLock::new(Broadcaster::new()));

    let me = broadcaster.clone();
    let broadcaster_delay = broadcaster.clone();
    let task_app_state = app_state.clone();

    HttpServer::new(move || {
        let generated_files = generate_files();
        let generated_css = generate_css();
        App::new()
            .register_data(app_state.clone())
            .register_data(broadcaster.clone())
            .service(index)
            .service(stream)
            .service(torrent_list)
            .service(actix_web_static_files::ResourceFiles::new(
                "/files",
                generated_files,
            ))
            .service(actix_web_static_files::ResourceFiles::new(
                "/css",
                generated_css,
            ))
    })
    .bind("127.0.0.1:8080")?
    .start();

    let task = Interval::new(Instant::now(), Duration::from_secs(10))
        .for_each(move |_| {
            eprintln!("timer event");
            let mut me = me.write().unwrap();
            let app_state = task_app_state.clone();
            if let Err(ok_clients) = me.message("data: ping\n\n") {
                eprintln!("refresh client list");
                me.clients = ok_clients;
            }
            Ok(())
        })
        .map_err(|_| ());
    Arbiter::spawn(task);

    test_incoming_rand_size_chunk(broadcaster_delay);

    system.run().map_err(|x| x.into())
}

fn test_incoming_rand_size_chunk(broadcaster: web::Data<RwLock<Broadcaster>>) {
    let mut rng = thread_rng();
    let millis = rng.gen_range(500u64, 1500u64);
    let task = Delay::new(Instant::now() + Duration::from_millis(millis))
        .and_then(move |_| {
            eprintln!("delayed task triggered after {}", millis);
            {
                let mut sender = broadcaster.write().unwrap();
                if let Err(ok_clients) = sender.message("data: pong\n\n") {
                    eprintln!("refresh client list");
                    sender.clients = ok_clients;
                }
            }
            test_incoming_rand_size_chunk(broadcaster);
            Ok(())
        })
        .map_err(|e| panic!("delay errored: {:?}", e));
    Arbiter::spawn(task);
}

struct Broadcaster {
    clients: Vec<Sender<Bytes>>,
}

impl Broadcaster {
    fn new() -> Self {
        Self { clients: vec![] }
    }

    fn new_client(&mut self) -> Client {
        eprintln!("adding new client");
        let (tx, rx) = channel(100);

        tx.clone()
            .try_send(Bytes::from("data: connected\n\n"))
            .unwrap();

        self.clients.push(tx);

        Client(rx)
    }

    fn message(&mut self, data: &str) -> Result<(), Vec<Sender<Bytes>>> {
        let mut ok_clients = vec![];
        eprintln!("message to {} client(s)", self.clients.len());
        for client in &mut self.clients {
            if let Ok(()) = client.try_send(Bytes::from(data)) {
                ok_clients.push(client.clone())
            }
        }
        if ok_clients.len() != self.clients.len() {
            return Err(ok_clients);
        }
        Ok(())
    }
}

// wrap Receiver in own type, with correct error type
struct Client(Receiver<Bytes>);

impl Stream for Client {
    type Item = Bytes;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        self.0.poll().map_err(ErrorInternalServerError)
    }
}
