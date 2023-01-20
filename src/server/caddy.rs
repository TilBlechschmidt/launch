use super::Algorithm;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

#[derive(Serialize, Clone)]
#[serde(into = "Value")]
pub struct CaddyConfig {
    pub http: HttpConfig,
    pub storage: Storage,
    pub tls: Option<TlsConfig>,
}

#[derive(Clone)]
pub struct TlsConfig {
    pub subjects: Vec<String>,
    pub email: String,
    pub token: String,
    pub staging: bool,
}

#[derive(Clone)]
pub struct HttpConfig {
    pub port: u16,
    pub hosts: Vec<HostConfig>,
    pub domains: Vec<String>,
}

/// Location where Caddy stores certificates and such
#[derive(Clone)]
pub struct Storage(pub PathBuf);

#[derive(Clone)]
pub struct HostConfig {
    pub hosts: Vec<String>,
    pub root: FileRoot,
    pub server: FileServer,
    pub fallback: Option<Fallback>,
}

/// Rewrites unmatched requests to the given path
#[derive(Clone)]
pub struct Fallback(pub String);

/// Sets the root for the match
#[derive(Clone)]
pub struct FileRoot(pub PathBuf);

/// Serves files and allows precompressed sidecars
#[derive(Clone)]
pub struct FileServer {
    pub compression: Vec<Algorithm>,
}

impl CaddyConfig {
    pub fn new(
        domains: Vec<String>,
        hosts: Vec<HostConfig>,
        storage_dir: PathBuf,
        tls: Option<TlsConfig>,
    ) -> Self {
        let port = if tls.is_some() { 443 } else { 80 };

        Self {
            http: HttpConfig {
                domains,
                hosts,
                port,
            },
            storage: Storage(storage_dir),
            tls,
        }
    }

    pub fn apply(&self, admin_url: &str) -> Result<(), ureq::Error> {
        ureq::post(&format!("{}/load", admin_url))
            .send_json(&self)
            .map(|_| ())
    }
}

impl HostConfig {
    pub fn new(
        hosts: Vec<String>,
        root: PathBuf,
        compression: Vec<Algorithm>,
        fallback: Option<String>,
    ) -> Self {
        Self {
            hosts,
            root: FileRoot(root),
            server: FileServer { compression },
            fallback: fallback.map(Fallback),
        }
    }
}

impl Into<Value> for CaddyConfig {
    fn into(self) -> Value {
        let storage: Value = self.storage.into();
        let http: Value = self.http.into();

        let mut apps = BTreeMap::new();
        apps.insert("http", http);

        if let Some(tls) = self.tls {
            apps.insert("tls", tls.into());
        }

        json!({
            "storage": storage,
            "apps": apps
        })
    }
}

impl Into<Value> for TlsConfig {
    fn into(self) -> Value {
        let ca = if self.staging {
            "https://acme-staging-v02.api.letsencrypt.org/directory"
        } else {
            "https://acme-v02.api.letsencrypt.org/directory"
        };

        json!({
            "automation": {
                "policies": [{
                    "subjects": self.subjects,
                    "issuers": [{
                        "module": "acme",
                        "email": self.email,
                        "ca": ca,
                        "challenges": {
                            "dns": {
                                "provider": {
                                    "name": "cloudflare",
                                    "api_token": self.token
                                },
                                "resolvers": ["1.1.1.1"]
                            }
                        }
                    }]
                }]
            }
        })
    }
}

impl Into<Value> for HttpConfig {
    fn into(self) -> Value {
        let routes: Vec<Value> = self.hosts.into_iter().map(Into::into).collect();

        json!({
            "servers": {
                "srv0": {
                    "listen": [format!(":{}", self.port)],
                    "routes": [{
                        "handle": [{
                            "handler": "subroute",
                            "routes": routes
                        }],
                        "match": [{
                            "host": self.domains
                        }],
                        "terminal": true
                    }]
                }
            }
        })
    }
}

impl Into<Value> for Storage {
    fn into(self) -> Value {
        json!({
            "module": "file_system",
            "root": self.0
        })
    }
}

impl Into<Value> for HostConfig {
    fn into(self) -> Value {
        let mut routes: Vec<Value> = vec![];

        routes.push(self.root.into());

        if let Some(fallback) = self.fallback {
            routes.push(fallback.into())
        }

        routes.push(self.server.into());

        json!({
            "handle": [{
                "handler": "subroute",
                "routes": routes
            }],
            "match": [{
                "host": self.hosts
            }]
        })
    }
}

impl Into<Value> for Fallback {
    fn into(self) -> Value {
        json!({
            "handle": [{
                "handler": "rewrite",
                "uri": "{http.matchers.file.relative}"
            }],
            "match": [{
                "file": {
                    "try_files": [
                        "{http.request.uri.path}",
                        "{http.request.uri.path}/index.html",
                        self.0
                    ]
                }
            }]
        })
    }
}

impl Into<Value> for FileRoot {
    fn into(self) -> Value {
        json!({
            "handle": [{
                "handler": "vars",
                "root": self.0
            }]
        })
    }
}

impl Into<Value> for FileServer {
    fn into(self) -> Value {
        let algorithms = self
            .compression
            .into_iter()
            .map(Algorithm::name)
            .collect::<Vec<_>>();

        let mut algorithms_map = HashMap::new();

        for algorithm in algorithms.iter() {
            algorithms_map.insert(algorithm, Value::Object(Map::new()));
        }

        json!({
            "handle": [{
                "handler": "file_server",
                "precompressed": algorithms_map,
                "precompressed_order": algorithms
            }]
        })
    }
}
