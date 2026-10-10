//! The contract test's HTTP client: `HttpProbe` over reqwest
//! (ADR-2610092329). One request, one answer; judging it is the use case's.

use async_trait::async_trait;
use hexa_analysis::ports::{HttpProbe, ProbeRequest, ProbeResponse};

pub struct ReqwestProbe {
    base: String,
    client: reqwest::Client,
}

impl ReqwestProbe {
    pub fn new(base_url: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_default();
        ReqwestProbe { base: base_url.trim_end_matches('/').to_string(), client }
    }
}

#[async_trait]
impl HttpProbe for ReqwestProbe {
    async fn send(&self, request: &ProbeRequest) -> Result<ProbeResponse, String> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes()).map_err(|e| e.to_string())?;
        let url = format!("{}{}", self.base, request.path_and_query);
        let mut builder = self.client.request(method, &url).header("Accept", "application/json");
        if let Some(body) = &request.body {
            builder = builder.header("Content-Type", "application/json").body(body.clone());
        }
        let resp = builder.send().await.map_err(|e| format!("{url}: {e}"))?;
        let status = resp.status().as_u16();
        let body = resp.text().await.map_err(|e| format!("{url}: reading the body: {e}"))?;
        Ok(ProbeResponse { status, body })
    }
}
