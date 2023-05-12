use brotli::enc::BrotliEncoderParams;
use flate2::{write::GzEncoder, Compression};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::File,
    io::{self, Seek},
    path::Path,
};
use walkdir::{DirEntry, WalkDir};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Statistics {
    /// Total bytes of all files combined
    pub size: u64,
    /// Number of bytes of compressible files only
    pub compressible: u64,
    /// Size of compressed files by algorithm
    pub compressed: HashMap<Algorithm, u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Algorithm {
    Gzip,
    Brotli,
}

pub struct Compressor {
    algorithms: Vec<Algorithm>,
    min_size: u64,
}

impl Compressor {
    pub fn algorithms(&self) -> Vec<Algorithm> {
        self.algorithms.clone()
    }

    pub fn compress(&self, dir: impl AsRef<Path>, filter: &[String]) -> io::Result<Statistics> {
        let mut total_size = 0;
        let mut total_compressible = 0;
        let mut total_compressed = HashMap::new();

        for entry in WalkDir::new(dir) {
            let entry = entry?;
            let size = entry.metadata()?.len();

            total_size += size;

            if size < self.min_size
                || !entry.file_type().is_file()
                || !match_extension(&entry, filter)
            {
                continue;
            }

            total_compressible += size;

            for algorithm in self.algorithms.iter() {
                let compressed = Compressor::apply(*algorithm, entry.path())?;
                total_compressed.insert(*algorithm, compressed);
            }
        }

        Ok(Statistics {
            size: total_size,
            compressible: total_compressible,
            compressed: total_compressed,
        })
    }

    fn apply(algorithm: Algorithm, path: impl AsRef<Path>) -> io::Result<u64> {
        let path = path.as_ref();
        let extension = path.extension().expect("matched file without extension");
        let destination_path = path.with_extension(format!(
            "{}.{}",
            extension
                .to_str()
                .expect("matched file with invalid extension"),
            algorithm.extension()
        ));

        let mut source = File::open(path)?;
        let mut destination = File::create(destination_path)?;

        algorithm.compress(&mut source, &mut destination)?;

        Ok(destination.stream_position()?)
    }
}

impl Default for Compressor {
    fn default() -> Self {
        use Algorithm::*;

        Compressor {
            algorithms: vec![Brotli, Gzip],
            min_size: 1_400,
        }
    }
}

impl Algorithm {
    pub fn name(self) -> &'static str {
        use Algorithm::*;

        match self {
            Gzip => "gzip",
            Brotli => "br",
        }
    }

    pub fn extension(self) -> &'static str {
        use Algorithm::*;

        match self {
            Gzip => "gz",
            Brotli => "br",
        }
    }

    fn compress(&self, source: &mut File, destination: &mut File) -> io::Result<()> {
        use Algorithm::*;

        match self {
            Gzip => {
                let mut encoder = GzEncoder::new(destination, Compression::best());
                io::copy(source, &mut encoder)?;
                encoder.finish()?;
            }
            Brotli => {
                let params = BrotliEncoderParams::default();
                brotli::BrotliCompress(source, destination, &params)?;
            }
        }

        Ok(())
    }
}

fn match_extension(entry: &DirEntry, extensions: &[String]) -> bool {
    if let Some(extension) = entry.path().extension() {
        for expected in extensions {
            if extension.eq_ignore_ascii_case(expected) {
                return true;
            }
        }
    }

    false
}
