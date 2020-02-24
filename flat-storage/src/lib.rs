use failure::Fail;
use serde::Serialize;
use std::{
    future::Future,
    ops::Deref,
    path::{Path, PathBuf},
};

#[derive(Debug, PartialEq, Clone, Serialize)]
pub struct FlatStorageFile {
    pub path: PathBuf,
    pub length: usize,
}

pub struct FlatStoragePieceIndex(usize);

impl From<usize> for FlatStoragePieceIndex {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl Deref for FlatStoragePieceIndex {
    type Target = usize;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Flat storage of different sized files.
///
/// Access to data is asynchronious. Files created lazy.
pub trait FlatStorage {
    fn files(&self) -> &[FlatStorageFile];

    fn read_piece<I: Into<FlatStoragePieceIndex>>(
        &self,
        index: I,
    ) -> Result<Option<Vec<u8>>, FlatStorageError>;

    fn write_piece<I: Into<FlatStoragePieceIndex>>(
        &self,
        index: I,
        block: Vec<u8>,
    ) -> Result<(), FlatStorageError>;
}

#[derive(Debug, Fail)]
pub enum FlatStorageError {
    #[fail(display = "cannot allocate file")]
    AllocateFile,
    #[fail(display = "cannot read block from file")]
    ReadBlock,
    #[fail(display = "cannot write block to file")]
    WriteBlock,
}
