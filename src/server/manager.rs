use super::{caddy::HostConfig, compressor::Compressor, storage::BundleStorage, Statistics};
use crate::{shared::Bundle, BundleConfig};
use std::{
    collections::HashMap,
    io::{self, ErrorKind},
};
use temp_dir::TempDir;
use ulid::Ulid;

#[derive(Debug, Clone)]
pub struct ActiveBundle {
    pub root: TempDir,
    pub config: BundleConfig,
    pub stats: Statistics,
}

#[derive(Debug)]
pub enum BundleStatus {
    Active(ActiveBundle),
    Failed(String),
}

pub struct BundleManager {
    bundles: HashMap<Ulid, BundleStatus>,

    pub storage: BundleStorage,
    compressor: Compressor,
}

impl BundleManager {
    pub fn new(storage: BundleStorage, compressor: Compressor) -> Self {
        Self {
            bundles: HashMap::new(),
            storage,
            compressor,
        }
    }

    pub fn bundles(&self) -> impl Iterator<Item = (Ulid, Bundle)> + '_ {
        self.bundles.iter().map(|(id, b)| (*id, Bundle::from(b)))
    }

    pub fn load_all(&mut self) -> io::Result<()> {
        for id in self.storage.enumerate()? {
            if let Err(e) = self.deploy(id) {
                self.bundles.insert(id, BundleStatus::Failed(e.to_string()));
            }
        }

        Ok(())
    }

    pub fn deploy(&mut self, id: Ulid) -> io::Result<Statistics> {
        let config = self.storage.metadata(id)?;
        let root = TempDir::with_prefix("launch-")?;
        let path = root.path();

        self.verify_bundle(id, &config)?;

        self.storage.unpack(id, path)?;
        let stats = self.compressor.compress(path, &config.compress)?;

        let bundle = ActiveBundle {
            root,
            config,
            stats: stats.clone(),
        };

        self.bundles.insert(id, BundleStatus::Active(bundle));

        Ok(stats)
    }

    fn verify_bundle(&self, id: Ulid, config: &BundleConfig) -> io::Result<()> {
        // TODO Verify that domain is allowed

        let active_domains = self
            .bundles
            .iter()
            .filter(|(i, _)| **i != id)
            .filter_map(|(_, status)| match status {
                BundleStatus::Active(bundle) => Some(&bundle.config.domain),
                _ => None,
            })
            .collect::<Vec<_>>();

        if active_domains.contains(&&config.domain) {
            return Err(io::Error::new(
                ErrorKind::Other,
                "domain already in use by another bundle",
            ));
        }

        Ok(())
    }

    pub fn remove(&mut self, id: Ulid) {
        self.bundles.remove(&id);
    }

    pub fn hosts(&self) -> impl Iterator<Item = HostConfig> + '_ {
        self.bundles.iter().filter_map(|(_, status)| match status {
            BundleStatus::Active(bundle) => Some(HostConfig::new(
                vec![bundle.config.domain.clone()],
                bundle.root.path().to_path_buf(),
                self.compressor.algorithms(),
                bundle.config.fallback.clone(),
            )),
            _ => None,
        })
    }

    pub fn domains(&self) -> impl Iterator<Item = String> + '_ {
        self.bundles.iter().filter_map(|(_, status)| match status {
            BundleStatus::Active(bundle) => Some(bundle.config.domain.clone()),
            _ => None,
        })
    }
}

impl From<&BundleStatus> for Bundle {
    fn from(value: &BundleStatus) -> Self {
        match value {
            BundleStatus::Active(b) => Self::Active {
                config: b.config.clone(),
                stats: b.stats.clone(),
            },
            BundleStatus::Failed(e) => Self::Failed { error: e.clone() },
        }
    }
}
