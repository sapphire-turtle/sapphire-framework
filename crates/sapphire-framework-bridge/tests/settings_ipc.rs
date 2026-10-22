//! `config.*` over the control plane: the bridge's settings, read and written by key.

mod common;

use sapphire_bridge_api::{ConfigEntry, EntryValue, Origin, Scope, SecretState, Slot, Value};
use sapphire_framework_bridge::LoopbackNetwork;

fn entry(key: &str, value: Value) -> ConfigEntry {
    ConfigEntry {
        key: key.to_owned(),
        value,
    }
}

fn s(text: &str) -> Value {
    Value::String(text.to_owned())
}

#[tokio::test]
async fn config_set_then_list_reports_the_value_and_its_origin() {
    let net = LoopbackNetwork::new();
    let host = common::start(&net, common::NODE_A, "host-a").await;
    let client = common::connect(&host).await;

    let schema = client.config_schema().await.unwrap();
    assert!(schema.settings.iter().any(|d| d.key == "net.discovery"));

    let result = client
        .config_set(
            Some(Scope::Device),
            vec![entry("net.discovery", Value::Bool(false))],
        )
        .await
        .unwrap();
    assert_eq!(result.changed, vec!["net.discovery".to_owned()]);

    let listed = client.config_list(None).await.unwrap();
    let found = listed
        .entries
        .iter()
        .find(|e| e.key == "net.discovery")
        .expect("listed");
    assert_eq!(found.value, EntryValue::Value(Value::Bool(false)));
    assert_eq!(found.origin, Origin::Scope(Scope::Device));
}

#[tokio::test]
async fn a_live_embedding_change_applies_without_restart() {
    let net = LoopbackNetwork::new();
    let host = common::start(&net, common::NODE_A, "host-a").await;
    let client = common::connect(&host).await;

    let result = client
        .config_set(
            None,
            vec![entry(
                "embed.local.model",
                s(sapphire_bridge_api::LOCAL_MODEL),
            )],
        )
        .await
        .unwrap();
    assert!(result.pending_restart.is_empty());

    let report = client.embed_settings().await.unwrap();
    assert!(
        report.models.local.is_some(),
        "the local slot is configured"
    );
    let _ = Slot::Local;
}

#[tokio::test]
async fn a_restart_key_is_reported_pending() {
    let net = LoopbackNetwork::new();
    let host = common::start(&net, common::NODE_A, "host-a").await;
    let client = common::connect(&host).await;

    let result = client
        .config_set(None, vec![entry("net.discovery", Value::Bool(false))])
        .await
        .unwrap();
    assert_eq!(result.pending_restart, vec!["net.discovery".to_owned()]);

    let listed = client.config_list(None).await.unwrap();
    let found = listed
        .entries
        .iter()
        .find(|e| e.key == "net.discovery")
        .unwrap();
    assert!(found.pending_restart);
}

#[tokio::test]
async fn a_rejected_batch_is_an_rpc_error_with_the_message() {
    let net = LoopbackNetwork::new();
    let host = common::start(&net, common::NODE_A, "host-a").await;
    let client = common::connect(&host).await;

    let err = client
        .config_set(None, vec![entry("no.such.key", Value::Bool(true))])
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("unknown setting `no.such.key`"),
        "{err}"
    );
    assert!(client.config_get("no.such.key").await.is_err());
}

#[tokio::test]
async fn secrets_never_come_back() {
    let net = LoopbackNetwork::new();
    let host = common::start(&net, common::NODE_A, "host-a").await;
    let client = common::connect(&host).await;

    client
        .config_set(None, vec![entry("embed.remote.key", s("sk-secret"))])
        .await
        .unwrap();
    let got = client.config_get("embed.remote.key").await.unwrap();
    assert_eq!(got.value, EntryValue::Secret(SecretState::Set));

    client
        .config_unset(None, vec!["embed.remote.key".into()])
        .await
        .unwrap();
    let got = client.config_get("embed.remote.key").await.unwrap();
    assert_eq!(got.value, EntryValue::Secret(SecretState::Unset));
}

#[tokio::test]
async fn a_dry_run_set_validates_and_writes_nothing() {
    let net = LoopbackNetwork::new();
    let host = common::start(&net, common::NODE_A, "host-a").await;
    let client = common::connect(&host).await;

    let result = client
        .config_check_set(
            Some(Scope::Device),
            vec![entry("net.discovery", Value::Bool(false))],
        )
        .await
        .unwrap();
    assert_eq!(result.changed, vec!["net.discovery".to_owned()]);

    let listed = client.config_list(None).await.unwrap();
    let found = listed
        .entries
        .iter()
        .find(|e| e.key == "net.discovery")
        .expect("listed");
    assert_ne!(
        found.origin,
        Origin::Scope(Scope::Device),
        "nothing was written"
    );
    assert!(!found.pending_restart);
}

#[tokio::test]
async fn a_dry_run_reports_the_error_a_write_would() {
    let net = LoopbackNetwork::new();
    let host = common::start(&net, common::NODE_A, "host-a").await;
    let client = common::connect(&host).await;

    let bad = vec![entry("net.discovery", s("not a bool"))];
    let written = client.config_set(None, bad.clone()).await.unwrap_err();
    let checked = client.config_check_set(None, bad).await.unwrap_err();
    assert_eq!(checked.to_string(), written.to_string());

    let unset = client
        .config_check_unset(None, vec!["no.such.key".into()])
        .await
        .unwrap_err();
    assert!(unset.to_string().contains("no.such.key"), "{unset}");
}

#[tokio::test]
async fn the_status_says_dry_run_is_supported() {
    let net = LoopbackNetwork::new();
    let host = common::start(&net, common::NODE_A, "host-a").await;
    let client = common::connect(&host).await;

    assert!(client.status().await.unwrap().config_dry_run);
}
