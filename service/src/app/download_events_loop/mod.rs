use super::*;
use std::path::PathBuf;

mod action;
mod add_torrent;
mod current_torrents;

use crate::storage::TorrentStorageState;
use action::torrent_action;
use add_torrent::add_torrent;
use current_torrents::save_current_torrents;
use download_torrent::TorrentDownloadState;

#[derive(Serialize, Clone)]
pub struct TorrentDownloadView {
    pub id: usize,
    pub name: String,
    pub received: usize,
    pub uploaded: usize,
    pub length: usize,
    pub active: bool,
}

#[derive(Clone)]
pub struct TorrentDownload {
    pub id: usize,
    pub name: String,
    pub header: TorrentDownloadHeader,
    pub process: Arc<TorrentProcess>,
    pub properties: Arc<Properties>,
    pub storage_state_watch: watch::Receiver<TorrentStorageState>,
    pub statistics_watch: watch::Receiver<TorrentDownloadState>,
}

impl From<&TorrentDownload> for TorrentDownloadView {
    fn from(torrent: &TorrentDownload) -> Self {
        let (uploaded, received) = {
            let storage_stage = torrent.storage_state_watch.borrow();
            (
                storage_stage.bytes_read as usize,
                storage_stage.bytes_write as usize,
            )
        };
        Self {
            id: torrent.id,
            name: torrent.name.clone(),
            active: torrent.header.state == TorrentDownloadStatus::Enabled,
            length: torrent.process.info.length,
            received,
            uploaded,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TorrentDownloadHeader {
    pub file: String,
    pub state: TorrentDownloadStatus,
}

#[derive(Clone, Serialize, Deserialize, Copy, PartialEq)]
pub enum TorrentDownloadStatus {
    Enabled,
    Disabled,
}

pub struct RsbtCommandAddTorrent {
    pub data: Vec<u8>,
    pub filename: String,
    pub state: TorrentDownloadStatus,
}

pub enum RsbtCommand {
    AddTorrent(RequestResponse<RsbtCommandAddTorrent, Result<TorrentDownload, RsbtError>>),
    TorrentHandshake {
        handshake_request: Handshake,
        handshake_sender: oneshot::Sender<Option<Arc<TorrentProcess>>>,
    },
    TorrentList(RequestResponse<(), Result<Vec<TorrentDownloadView>, RsbtError>>),
    TorrentAction(RequestResponse<RsbtCommandTorrentAction, Result<(), RsbtError>>),
}

pub(crate) async fn download_events_loop(
    properties: Arc<Properties>,
    mut events: Receiver<RsbtCommand>,
) {
    let mut torrents = vec![];
    let mut id = 0;

    while let Some(event) = events.next().await {
        match event {
            RsbtCommand::AddTorrent(request_response) => {
                debug!("add torrent");
                let torrent = add_torrent(
                    properties.clone(),
                    request_response.request(),
                    &mut id,
                    &mut torrents,
                )
                .await;
                if let Err(err) = request_response.response(torrent) {
                    error!("cannot send response for add torrent: {}", err);
                }
            }
            RsbtCommand::TorrentHandshake {
                handshake_request,
                handshake_sender,
            } => {
                debug!("searching for matching torrent handshake");

                let hash_id = handshake_request.info_hash;

                if handshake_sender
                    .send(
                        torrents
                            .iter()
                            .map(|x| &x.process)
                            .find(|x| x.hash_id == hash_id)
                            .cloned(),
                    )
                    .is_err()
                {
                    error!("cannot send handshake, receiver is dropped");
                }
            }
            RsbtCommand::TorrentList(request_response) => {
                debug!("collecting torrent list");
                let torrents_view = torrents.iter().map(TorrentDownloadView::from).collect();
                if let Err(err) = request_response.response(Ok(torrents_view)) {
                    error!("cannot send response for torrent list: {}", err);
                }
            }
            RsbtCommand::TorrentAction(request_response) => {
                debug!("torrent action");

                let response = torrent_action(request_response.request(), &mut torrents).await;

                if let Err(err) = request_response.response(response) {
                    error!("cannot send response for add torrent: {}", err);
                }
            }
        }
    }

    debug!("download_events_loop done");
}
