use super::*;

#[get("/upload")]
async fn upload_form(_user: User) -> impl Responder {
    HttpResponse::Ok()
        .content_type("text/html")
        .body(include_str!("../static/upload.html"))
}

#[post("/upload")]
async fn upload(
    _user: User,
    event_sender: web::Data<Mutex<Sender<RsbtCommand>>>,
    mut payload: Multipart,
) -> Result<HttpResponse, Error> {
    if let Some(item) = payload.next().await {
        let mut field = item?;
        let content_type = field.content_disposition().unwrap();
        let filename = content_type.get_filename().unwrap();

        let mut torrent = vec![];
        while let Some(chunk) = field.next().await {
            let data = chunk?;
            torrent.extend(&data);
        }

        let (request_response, receiver) = RequestResponse::new(RsbtCommandAddTorrent {
            data: torrent,
            filename: filename.to_string(),
            state: TorrentDownloadState::Enabled,
        });
        {
            let mut event_sender = event_sender.lock().await;
            if let Err(err) = event_sender
                .send(RsbtCommand::AddTorrent(request_response))
                .await
            {
                error!("cannot send to torrent process: {}", err);
                return Ok(HttpResponse::InternalServerError().json(Failure {
                    error: format!("cannot send to torrent process: {}", err),
                }));
            }
        }

        return Ok(match receiver.await {
            Ok(Ok(torrent)) => HttpResponse::Ok().json(BackendTorrentDownload {
                id: torrent.id,
                name: torrent.name.as_str().into(),
                received: 0,
                uploaded: 0,
                length: torrent.process.info.length,
                active: true,
            }),
            Ok(Err(err)) => {
                error!("error in update call: {}", err);
                HttpResponse::InternalServerError().json(Failure {
                    error: format!("cannot add torrent process: {}", err),
                })
            }
            Err(err) => {
                error!("error in receiver: {}", err);
                HttpResponse::InternalServerError().json(Failure {
                    error: format!("cannot receive from add torrent process: {}", err),
                })
            }
        });
    }
    Ok(HttpResponse::UnprocessableEntity().into())
}
