use super::*;

mod process_announce;
mod process_peer_announced;
mod process_peer_connected;
mod process_peer_forwarded;
mod process_peer_interested;
mod process_peer_piece;
mod process_peer_piece_canceled;
mod process_peer_piece_downloaded;
mod process_peer_piece_request;
mod process_peer_pieces;
mod process_peer_unchoke;

use process_announce::process_announce;
use process_peer_announced::process_peer_announced;
use process_peer_connected::process_peer_connected;
use process_peer_forwarded::process_peer_forwarded;
use process_peer_interested::process_peer_interested;
use process_peer_piece::process_peer_piece;
use process_peer_piece_canceled::process_peer_piece_canceled;
use process_peer_piece_downloaded::process_peer_piece_downloaded;
use process_peer_piece_request::process_peer_piece_request;
use process_peer_pieces::process_peer_pieces;
use process_peer_unchoke::process_peer_unchoke;

#[derive(Debug)]
pub(crate) enum DownloadTorrentEvent {
    Announce(Vec<Peer>),
    PeerAnnounced(Peer),
    PeerConnected(Uuid, TcpStream),
    PeerForwarded(TcpStream),
    PeerConnectFailed(Uuid),
    PeerDisconnect(Uuid),
    PeerPieces(Uuid, Vec<u8>),
    PeerPiece(Uuid, usize),
    PeerUnchoke(Uuid),
    PeerInterested(Uuid),
    PeerPieceDownloaded(Uuid, Vec<u8>),
    PeerPieceCanceled(Uuid),
    PeerPieceRequest {
        peer_id: Uuid,
        index: u32,
        begin: u32,
        length: u32,
    },
}

impl Display for DownloadTorrentEvent {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            DownloadTorrentEvent::PeerPieceDownloaded(uuid, data) => {
                write!(f, "PeerPieceDownloaded({}, [{}])", uuid, data.len())
            }
            _ => write!(f, "{:?}", self),
        }
    }
}

pub(crate) async fn download_torrent(
    settings: Arc<Settings>,
    torrent_process: Arc<TorrentProcess>,
    mut broker_receiver: Receiver<DownloadTorrentEvent>,
) -> Result<(), RsbtError> {
    let mut torrent_storage = TorrentStorage::new(settings.clone(), torrent_process.clone());

    let (abort_handle, abort_registration) = AbortHandle::new_pair();

    let announce_loop = Abortable::new(
        announce::announce_loop(settings.clone(), torrent_process.clone()).map_err(|e| {
            error!("announce loop error: {}", e);
            e
        }),
        abort_registration,
    )
    .map_err(|e| {
        error!("abortable error: {}", e);
        e.into()
    });

    let mut peer_states = HashMap::new();
    let mut mode = TorrentDownloadMode::Normal;

    let download_torrent_events_loop = async move {
        while let Some(event) = broker_receiver.next().await {
            debug!("received event: {}", event);
            match event {
                DownloadTorrentEvent::Announce(peers) => {
                    debug!("we got announce, what now?");
                    spawn_and_log_error(process_announce(torrent_process.clone(), peers), || {
                        "process announce failed".to_string()
                    });
                }
                DownloadTorrentEvent::PeerAnnounced(peer) => {
                    debug!("peer announced: {:?}", peer);
                    if let Err(err) = process_peer_announced(
                        torrent_process.clone(),
                        &mut peer_states,
                        peer.clone(),
                    )
                    .await
                    {
                        error!("cannot process peer announced {:?}: {}", peer, err);
                    }
                }
                DownloadTorrentEvent::PeerDisconnect(peer_id) => {
                    if let Some(_peer_state) = peer_states.remove(&peer_id) {
                        debug!("[{}] removed peer due to disconnect", peer_id);
                    }
                }
                DownloadTorrentEvent::PeerConnectFailed(peer_id) => {
                    if let Some(_peer_state) = peer_states.remove(&peer_id) {
                        debug!("[{}] removed peer due to connection failure", peer_id);
                    }
                }
                DownloadTorrentEvent::PeerForwarded(stream) => {
                    debug!("peer forwarded");
                    if let Err(err) = process_peer_forwarded(
                        torrent_process.clone(),
                        &mut peer_states,
                        stream,
                        &mut torrent_storage,
                    )
                    .await
                    {
                        error!("cannot forward peer: {}", err);
                    }
                }
                DownloadTorrentEvent::PeerConnected(peer_id, stream) => {
                    debug!("[{}] peer connected", peer_id);
                    if let Err(err) = process_peer_connected(
                        torrent_process.clone(),
                        &mut peer_states,
                        peer_id,
                        stream,
                    )
                    .await
                    {
                        error!("[{}] cannot process peer connected: {}", peer_id, err);
                    }
                }
                DownloadTorrentEvent::PeerPiece(peer_id, piece) => {
                    debug!("[{}] peer piece: {}", peer_id, piece);
                    if let Err(err) = process_peer_piece(
                        &mut peer_states,
                        &mode,
                        peer_id,
                        piece,
                        &mut torrent_storage,
                    )
                    .await
                    {
                        error!("[{}] cannot process peer piece: {}", peer_id, err);
                    }
                }
                DownloadTorrentEvent::PeerPieces(peer_id, pieces) => {
                    debug!("[{}] peer pieces", peer_id);
                    if let Err(err) = process_peer_pieces(
                        &mut peer_states,
                        &mode,
                        peer_id,
                        pieces,
                        &mut torrent_storage,
                    )
                    .await
                    {
                        error!("[{}] cannot process peer pieces: {}", peer_id, err);
                    }
                }
                DownloadTorrentEvent::PeerUnchoke(peer_id) => {
                    debug!("[{}] peer unchoke", peer_id);
                    if let Err(err) = process_peer_unchoke(&mut peer_states, peer_id).await {
                        error!("[{}] cannot process peer unchoke: {}", peer_id, err);
                    }
                }
                DownloadTorrentEvent::PeerInterested(peer_id) => {
                    debug!("[{}] peer interested", peer_id);
                    if let Err(err) = process_peer_interested(&mut peer_states, peer_id).await {
                        error!("[{}] cannot process peer interested: {}", peer_id, err);
                    }
                }
                DownloadTorrentEvent::PeerPieceCanceled(peer_id) => {
                    debug!("[{}] canceled piece for peer", peer_id);
                    if let Err(err) = process_peer_piece_canceled(
                        &mut peer_states,
                        &mode,
                        peer_id,
                        &mut torrent_storage,
                    )
                    .await
                    {
                        error!("[{}] cannot process peer piece canceled: {}", peer_id, err);
                    }
                }
                DownloadTorrentEvent::PeerPieceDownloaded(peer_id, piece) => {
                    debug!("[{}] downloaded piece for peer", peer_id);
                    if let Err(err) = process_peer_piece_downloaded(
                        &mut peer_states,
                        &mode,
                        peer_id,
                        piece,
                        &mut torrent_storage,
                    )
                    .await
                    {
                        error!(
                            "[{}] cannot process peer piece downloaded: {}",
                            peer_id, err
                        );
                    }

                    mode = determine_download_mode(&mut peer_states, &mut torrent_storage, peer_id);

                    let pieces_left = torrent_storage.receiver.borrow().pieces_left;
                    if pieces_left == 0 {
                        debug!(
                            "torrent downloaded, hash: {}",
                            percent_encode(&torrent_process.hash_id, NON_ALPHANUMERIC)
                        );
                    } else {
                        debug!("pieces left: {}", pieces_left);
                    }
                }
                DownloadTorrentEvent::PeerPieceRequest {
                    peer_id,
                    index,
                    begin,
                    length,
                } => {
                    debug!("[{}] request piece to peer", peer_id);
                    if let Err(err) = process_peer_piece_request(
                        &mut peer_states,
                        peer_id,
                        index,
                        begin,
                        length,
                        &mut torrent_storage,
                    )
                    .await
                    {
                        error!("[{}] cannot process peer piece request: {}", peer_id, err);
                    }
                }
            }
        }

        abort_handle.abort();

        debug!("download events loop is done");

        Ok::<(), RsbtError>(())
    };

    match try_join!(announce_loop, download_torrent_events_loop) {
        Ok(_) | Err(RsbtError::Aborted) => debug!("download torrent is done"),
        Err(e) => error!("download torrent finished with failure: {}", e),
    };

    debug!("download_torrent done");

    Ok(())
}