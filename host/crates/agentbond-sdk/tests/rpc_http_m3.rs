use std::time::Duration;

use agentbond_sdk::HttpChainReader;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn rejects_credentialed_rpc_url() {
    let err = HttpChainReader::new("http://user:pass@127.0.0.1:8899", Duration::from_secs(1));
    assert!(err.is_err());
}

#[tokio::test]
async fn get_genesis_hash_ok() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc":"2.0","id":1,"result":"5eykt4UsLoyBJaSfb9PRppPPjqSV4kt3Kg8ndRVYMQ"
        })))
        .mount(&server)
        .await;
    let rpc = HttpChainReader::new(server.uri(), Duration::from_secs(2)).expect("client");
    let genesis = rpc.get_genesis_hash().await.expect("genesis");
    assert_eq!(genesis, "5eykt4UsLoyBJaSfb9PRppPPjqSV4kt3Kg8ndRVYMQ");
}

#[tokio::test]
async fn rejects_redirect_and_oversized_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/elsewhere"))
        .mount(&server)
        .await;
    let rpc = HttpChainReader::new(server.uri(), Duration::from_secs(2)).expect("client");
    let err = rpc.get_genesis_hash().await.expect_err("redirect");
    assert!(err.to_string().contains("redirect"));
}

#[tokio::test]
async fn rejects_oversized_response() {
    let server = MockServer::start().await;
    let huge = "A".repeat(300_000);
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(huge))
        .mount(&server)
        .await;
    let rpc = HttpChainReader::new(server.uri(), Duration::from_secs(2)).expect("client");
    let err = rpc.get_genesis_hash().await.expect_err("large");
    assert!(err.to_string().contains("too large") || err.to_string().contains("rpc"));
}
