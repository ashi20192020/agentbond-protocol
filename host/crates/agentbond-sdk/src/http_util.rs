use url::Url;

use crate::error::SdkError;

pub fn reject_credentialed_url(raw: &str) -> Result<Url, SdkError> {
    let url = Url::parse(raw).map_err(|e| SdkError::Rpc(format!("invalid url: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(SdkError::Rpc("url must be http(s)".into()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(SdkError::Rpc("url must not contain credentials".into()));
    }
    Ok(url)
}

pub fn build_http_client(timeout: std::time::Duration) -> Result<reqwest::Client, SdkError> {
    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| SdkError::Rpc(e.to_string()))
}

/// Read a response body with a hard upper bound while streaming.
pub async fn read_body_bounded(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<bytes::Bytes, SdkError> {
    use futures_util::StreamExt;
    let mut out = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| SdkError::Rpc(e.to_string()))?;
        if out.len().saturating_add(chunk.len()) > max_bytes {
            return Err(SdkError::Rpc("response body too large".into()));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(bytes::Bytes::from(out))
}
