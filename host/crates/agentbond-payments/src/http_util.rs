use std::time::Duration;

use url::Url;

use crate::error::PaymentError;

pub const MAX_FACILITATOR_BODY: usize = 64 * 1024;

pub fn reject_credentialed_url(raw: &str) -> Result<Url, PaymentError> {
    let url = Url::parse(raw).map_err(|e| PaymentError::Config(format!("invalid url: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(PaymentError::Config("url must be http(s)".into()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(PaymentError::Config(
            "url must not contain credentials".into(),
        ));
    }
    Ok(url)
}

pub fn build_http_client(timeout: Duration) -> Result<reqwest::Client, PaymentError> {
    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| PaymentError::Facilitator(e.to_string()))
}

pub async fn read_body_bounded(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<bytes::Bytes, PaymentError> {
    use futures_util::StreamExt;
    let mut out = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| PaymentError::Facilitator(e.to_string()))?;
        if out.len().saturating_add(chunk.len()) > max_bytes {
            return Err(PaymentError::Facilitator("response body too large".into()));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(bytes::Bytes::from(out))
}
