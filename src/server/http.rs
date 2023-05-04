use super::{
    caddy::CaddyConfig, compressor::Compressor, manager::BundleManager, storage::BundleStorage,
    Options,
};
use std::{
    collections::HashMap,
    io::{self, ErrorKind},
    process::Command,
    thread::sleep,
    time::Duration,
};
use tiny_http::{Method, Request, Response};
use ulid::Ulid;

const INGRESS_UPDATE_SCRIPT: &str = r#"
echo "Applying new ingress manifest"
kubectl apply -f $INGRESS_PATH

OLD_INGRESS=$(kubectl get ingress -o=jsonpath="{.items[?(@.metadata.annotations.dev\.blechschmidt\.launch/deploy-id!=\"$DEPLOY_ID\")].metadata.name}")

echo "Deleting stale ingress resources: '$OLD_INGRESS'"
kubectl delete ingress $OLD_INGRESS
"#;

pub struct Server {
    options: Options,
    manager: BundleManager,
}

impl Server {
    pub fn new(options: Options) -> io::Result<Self> {
        let storage = BundleStorage::new(options.storage.clone())?;
        let manager = BundleManager::new(storage, Compressor::default());
        let mut instance = Self { options, manager };

        instance.manager.load_all()?;
        instance.reload_config()?;
        instance.reload_ingress()?;

        Ok(instance)
    }

    fn reload_config(&self) -> io::Result<()> {
        let hosts = self.manager.hosts().collect::<Vec<_>>();
        let config = CaddyConfig::new(
            self.options.domains.clone(),
            hosts,
            self.options.caddy_dir.clone(),
            self.options.tls.clone(),
        );

        let mut result = Ok(());
        for _ in 0..10 {
            result = config
                .apply(&self.options.caddy_endpoint)
                .map_err(|e| io::Error::new(ErrorKind::Other, e));

            if result.is_ok() {
                return Ok(());
            }

            sleep(Duration::from_millis(250));
        }

        result
    }

    fn reload_ingress(&self) -> io::Result<()> {
        if let Some(service) = &self.options.kube_service {
            let deploy_id = Ulid::new().to_string();

            let ingresses = self
                .manager
                .domains()
                .map(|domain| {
                    format!(
                        r#"
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: launch-{domain}
  annotations:
    dev.blechschmidt.launch/deploy-id: {deploy_id}
spec:
  rules:
  - http:
      paths:
      - path: /
        pathType: Prefix
        backend:
          service:
            name: {service}
            port:
              number: 80
---
            "#,
                        domain = domain,
                        service = service,
                        deploy_id = &deploy_id
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");

            let dir = temp_dir::TempDir::new()?;
            let path = dir.child("ingresses.yml");
            std::fs::write(&path, ingresses.as_bytes())?;

            let status = Command::new("/bin/sh")
                .args(["-c", INGRESS_UPDATE_SCRIPT])
                .env("INGRESS_PATH", path)
                .env("DEPLOY_ID", deploy_id)
                .spawn()?
                .wait()?;

            if !status.success() {
                eprintln!("Failed to run ingress update script");
            }
        }

        Ok(())
    }

    pub fn listen(&mut self, port: u16) {
        use Method::*;

        let server = tiny_http::Server::http(("0.0.0.0", port)).expect("failed to bind");

        for mut request in server.incoming_requests() {
            let response = if *request.method() == Get {
                Response::from_string(self.handle_get())
            } else if let Some(Ok(id)) = request
                .url()
                .strip_prefix("/bundle/")
                .map(Ulid::from_string)
            {
                let result = match request.method() {
                    Post => self.handle_post(&mut request, id),
                    Delete => self.handle_delete(&mut request, id),
                    _ => Ok("OK".into()),
                };

                match result {
                    Ok(payload) => Response::from_string(payload),
                    Err(e) => Response::from_string(e.to_string()).with_status_code(500),
                }
            } else {
                Response::from_string("Not found").with_status_code(404)
            };

            request.respond(response).ok();
        }
    }

    fn handle_get(&self) -> String {
        let map = self.manager.bundles().collect::<HashMap<_, _>>();
        serde_json::to_string(&map).expect("failed to serialize bundles")
    }

    fn handle_post(&mut self, request: &mut Request, id: Ulid) -> io::Result<String> {
        self.manager.storage.add(id, request.as_reader())?;
        let bundle = self.manager.deploy(id)?;
        self.reload_config()?;
        self.reload_ingress()?;
        Ok(serde_json::to_string(&bundle)?)
    }

    fn handle_delete(&mut self, _request: &mut Request, id: Ulid) -> io::Result<String> {
        self.manager.storage.remove(id)?;
        self.manager.remove(id);
        self.reload_config()?;
        self.reload_ingress()?;
        Ok("Deleted".into())
    }
}
