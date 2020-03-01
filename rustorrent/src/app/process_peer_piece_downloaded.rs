use super::*;

pub(crate) async fn process_peer_piece_downloaded(
    peer_states: &mut HashMap<Uuid, PeerState>,
    mode: &TorrentDownloadMode,
    peer_id: Uuid,
    piece: Vec<u8>,
    storage: &mut TorrentStorage,
) -> Result<(), RustorrentError> {
    debug!("[{}] peer piece downloaded", peer_id);

    let (index, new_pieces) = if let Some(existing_peer) = peer_states.get_mut(&peer_id) {
        if let TorrentPeerState::Connected {
            ref pieces,
            ref mut downloading_piece,
            ref mut downloading_since,
            ..
        } = existing_peer.state
        {
            if let (Some(index), Some(_since)) =
                (downloading_piece.take(), downloading_since.take())
            {
                storage.save(index, piece).await?;

                let mut downloadable = vec![];
                for (i, &a) in pieces.iter().enumerate() {
                    match_pieces(
                        &mut downloadable,
                        &storage.receiver.borrow().downloaded,
                        i,
                        a,
                    );
                }
                (index, downloadable)
            } else {
                return Ok(());
            }
        } else {
            return Ok(());
        }
    } else {
        return Ok(());
    };

    for (peer_id, peer_state) in peer_states.iter_mut().filter(|(&key, _)| key != peer_id) {
        if let TorrentPeerState::Connected {
            ref mut sender,
            ref pieces,
            ref mut downloading_piece,
            ..
        } = peer_state.state
        {
            let peer_already_have_piece = bit_by_index(index, pieces).is_some();
            if peer_already_have_piece {
                continue;
            }
            debug!("[{}] sending Have {}", peer_id, index);
            if let Err(err) = sender.send(PeerMessage::Have(index)).await {
                error!(
                    "[{}] cannot send Have to {:?}: {}",
                    peer_id, peer_state.peer, err
                );
            };

            let peer_downloads_same_piece = *downloading_piece == Some(index);
            if peer_downloads_same_piece {
                if let Err(err) = sender.send(PeerMessage::Cancel).await {
                    error!(
                        "[{}] cannot send Have to {:?}: {}",
                        peer_id, peer_state.peer, err
                    );
                };
            }
        }
    }

    select_new_peer(&new_pieces, peer_states, mode, peer_id).await?;

    Ok(())
}
