use super::*;

pub(crate) async fn connect_to_peer(
    settings: Arc<Settings>,
    torrent_process: Arc<TorrentProcess>,
    peer_id: Uuid,
    peer: Peer,
) -> Result<(), RustorrentError> {
    let socket_addr = SocketAddr::new(peer.ip, peer.port);
    let mut stream = TcpStream::connect(socket_addr).await?;

    stream.write_all(&torrent_process.handshake).await?;

    let mut handshake_reply = vec![0u8; 68];

    stream.read_exact(&mut handshake_reply).await?;

    let handshake_reply: Handshake = handshake_reply.try_into()?;

    if handshake_reply.info_hash != torrent_process.hash_id {
        error!("[{}] peer {:?}: hash is wrong. Disconnect.", peer_id, peer);
        torrent_process
            .broker_sender
            .clone()
            .send(DownloadTorrentEvent::PeerConnectFailed(peer_id))
            .await?;
        return Ok(());
    }

    torrent_process
        .broker_sender
        .clone()
        .send(DownloadTorrentEvent::PeerConnected(peer_id, stream))
        .await?;

    Ok(())
}
