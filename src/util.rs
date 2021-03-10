use anyhow::Result;
use k8s_openapi::api::core::v1::Pod;
use kube::{api::ListParams, Api, Client, Config};

pub async fn get_pods(namespace: &str) -> Result<Vec<Pod>> {
    let mut client_config = Config::infer().await?;
    // client_config.timeout = std::time::Duration::from_secs(60 * 60 * 24);
    client_config.timeout = None;
    let client = Client::new(client_config);

    let pods: Api<Pod> = Api::namespaced(client, namespace);
    let mut lp = ListParams::default();
    lp.timeout = None;

    Ok(pods.list(&lp).await?.into_iter().collect())
}
