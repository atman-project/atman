use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::{
    actors::sync::Error,
    doc::{DocId, DocSpace},
};

const FILE_EXT: &str = "dat";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub syncman_dir: PathBuf,
}

impl Config {
    pub(crate) fn create_dirs(&self) -> Result<(), Error> {
        std::fs::create_dir_all(&self.syncman_dir).map_err(|e| Error::IO {
            message: "Failed to create syncman directory".to_string(),
            cause: e,
        })?;
        info!("Created (or checked) syncman dir: {:?}", self.syncman_dir);
        Ok(())
    }

    pub(crate) fn syncman_path(&self, doc_space: &DocSpace, doc_id: &DocId) -> PathBuf {
        self.syncman_dir
            .join(doc_space.as_str())
            .join(format!("{}.{FILE_EXT}", doc_id.as_str()))
    }

    /// Returns all syncman files in the syncman directory.
    ///
    /// The directory structure is as follows:
    /// /syncman_dir
    ///             /doc-space-0
    ///                         /doc-id-0.dat
    ///                         /doc-id-1.dat
    pub(crate) fn syncman_paths(&self) -> Result<Vec<(DocSpace, DocId, PathBuf)>, Error> {
        let mut paths = vec![];

        let entries = std::fs::read_dir(&self.syncman_dir).map_err(|cause| Error::IO {
            message: format!(
                "failed to read syncman directory: {}",
                self.syncman_dir.display()
            ),
            cause,
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(dirname) = path.file_name() else {
                continue;
            };
            let Some(doc_space) = dirname.to_str().map(DocSpace::from) else {
                continue;
            };
            let subentries = std::fs::read_dir(&path).map_err(|cause| Error::IO {
                message: format!("failed to read directory: {}", path.display()),
                cause,
            })?;
            for subentry in subentries.flatten() {
                let subpath = subentry.path();
                if !subpath.is_file()
                    || subpath.extension().and_then(|s| s.to_str()) != Some(FILE_EXT)
                {
                    continue;
                }
                let Some(stem) = subpath.file_stem() else {
                    continue;
                };
                if let Some(doc_id) = stem.to_str().map(DocId::from) {
                    paths.push((doc_space.clone(), doc_id, subpath));
                }
            }
        }

        Ok(paths)
    }
}
