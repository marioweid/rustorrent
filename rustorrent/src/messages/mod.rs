use super::*;

use std::sync::Arc;
use std::sync::Mutex;

use futures::prelude::*;
use futures::sync::mpsc::{channel, Receiver, Sender};
use log::{debug, error, info, warn};

use crate::app::*;
use crate::errors::{RustorrentError, TryFromBencode};
use crate::types::message::Message;

mod bitfield;
mod piece;
mod unchoke;

pub(crate) use bitfield::message_bitfield;
pub(crate) use piece::message_piece;
pub(crate) use unchoke::message_unchoke;

#[inline]
fn index_in_bitarray(index: usize) -> (usize, u8) {
    (index / 8, 128 >> (index % 8))
}

#[inline]
fn bit_by_index(index: usize, data: &[u8]) -> Option<(usize, u8)> {
    let (index_byte, index_bit) = index_in_bitarray(index);
    data.get(index_byte).and_then(|&v| {
        if v & index_bit == index_bit {
            Some((index_byte, index_bit))
        } else {
            None
        }
    })
}

pub(crate) fn send_message_to_peer(sender: &Sender<Message>, message: Message) {
    let conntx = sender.clone();
    tokio::spawn(
        conntx
            .send(message)
            .map(|_| ())
            .map_err(|err| error!("Cannot send message: {}", err)),
    );
}