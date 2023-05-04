mod caddy;
mod compressor;
mod http;
mod manager;
mod storage;

use caddy::TlsConfig;
use http::Server;
use std::path::PathBuf;

pub use compressor::{Algorithm, Statistics};

pub struct Options {
    storage: PathBuf,
    domains: Vec<String>,

    caddy_dir: PathBuf,
    caddy_endpoint: String,

    tls: Option<TlsConfig>,
    kube_service: Option<String>,
}

pub fn run() -> anyhow::Result<()> {
    let options = Options::default();
    let mut server = Server::new(options).expect("failed to create server");

    println!("Listening on 0.0.0.0:8088");
    server.listen(8088);

    Ok(())
}

impl Default for Options {
    fn default() -> Self {
        Options {
            kube_service: Some(
                std::env::var("LAUNCH_SERVICE").expect("Kubernetes service name not found in env"),
            ),

            storage: "/var/www/bundles".into(),
            domains: vec![
                "blechschmidt.de".into(),
                "blechschmidt.dev".into(),
                "groundtrack.app".into(),
                "ucdaero.space".into(),
                "*.blechschmidt.de".into(),
                "*.blechschmidt.dev".into(),
                "*.groundtrack.app".into(),
                "*.ucdaero.space".into(),
            ],

            caddy_dir: "/etc/caddy".into(),
            caddy_endpoint: "http://localhost:2019".into(),

            tls: None,
        }
    }
}
