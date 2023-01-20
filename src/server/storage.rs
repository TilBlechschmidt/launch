use crate::BundleConfig;
use std::{
    fs::{create_dir_all, read_dir, remove_file, File},
    io::{self, ErrorKind, Read},
    path::{Path, PathBuf},
};
use tar::Archive;
use ulid::Ulid;

pub struct BundleStorage(PathBuf);

impl BundleStorage {
    pub fn new(root: PathBuf) -> io::Result<Self> {
        create_dir_all(&root)?;
        Ok(Self(root))
    }

    fn bundle_path(&self, id: Ulid) -> PathBuf {
        self.0.join(format!("{}.launch", id.to_string()))
    }

    pub fn remove(&self, id: Ulid) -> io::Result<()> {
        match remove_file(self.bundle_path(id)) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub fn add(&self, id: Ulid, data: &mut dyn Read) -> io::Result<()> {
        let mut file = File::create(self.bundle_path(id))?;
        io::copy(data, &mut file)?;
        file.sync_all()?;
        Ok(())
    }

    pub fn enumerate(&self) -> io::Result<Vec<Ulid>> {
        let mut bundles = Vec::new();

        for entry in read_dir(&self.0)? {
            let entry = entry?;

            if entry.file_type()?.is_file()
                && entry
                    .path()
                    .extension()
                    .map(|e| e.eq_ignore_ascii_case("launch"))
                    .unwrap_or_default()
            {
                if let Some(Ok(id)) = entry
                    .path()
                    .file_stem()
                    .map(|s| s.to_str())
                    .flatten()
                    .map(Ulid::from_string)
                {
                    bundles.push(id)
                } else {
                    eprintln!("skipping unknown file @ {:?}", entry.path());
                }
            }
        }

        Ok(bundles)
    }

    pub fn metadata(&self, id: Ulid) -> io::Result<BundleConfig> {
        let file = File::open(&self.bundle_path(id))?;
        let mut archive = Archive::new(file);

        for entry in archive.entries()? {
            let mut entry = entry?;

            if entry.path()?.ends_with("launch.config") {
                let options: BundleConfig = serde_json::from_reader(&mut entry)?;
                return Ok(options);
            }
        }

        Err(io::Error::new(
            ErrorKind::NotFound,
            "no launch config found",
        ))
    }

    pub fn unpack(&self, id: Ulid, destination: impl AsRef<Path>) -> io::Result<()> {
        let mut archive = Archive::new(File::open(&self.bundle_path(id))?);
        create_dir_all(&destination)?;
        archive.set_overwrite(true);
        archive.unpack(&destination)?;
        Ok(())
    }
}
