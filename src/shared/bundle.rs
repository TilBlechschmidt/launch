use crate::server::Statistics;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BundleConfig {
    /// Friendly name for the bundle
    pub name: String,

    /// Where the page will be available
    pub domain: String,

    /// File extensions which should be precompressed
    #[serde(default)]
    pub compress: Vec<String>,

    /// Fallback path for serving single-page applications
    pub fallback: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Bundle {
    Active {
        config: BundleConfig,
        stats: Statistics,
    },
    Failed {
        error: String,
    },
}
