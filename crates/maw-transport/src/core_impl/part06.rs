use std::collections::BTreeMap;

/// Generic bounded HTTP request used by the WASM host-function layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Option<String>,
    pub timeout_ms: Option<u64>,
    pub follow_redirects: bool,
}

/// Generic HTTP response returned to WASM plugins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: String,
    pub url: String,
}

impl ReqwestHttpTransportIo {
    /// Execute a generic HTTP request using the existing reqwest/rustls transport client.
    ///
    /// # Errors
    ///
    /// Returns request-construction, network, or body-read errors as strings.
    pub async fn request(&self, request: &HttpRequest) -> Result<HttpResponse, String> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|error| format!("invalid HTTP method: {error}"))?;
        let timeout = request.timeout_ms.unwrap_or(self.timeout_ms());
        let client = if request.follow_redirects {
            self.client.clone()
        } else {
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_millis(timeout))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|error| format!("http client build failed: {error}"))?
        };
        let mut builder = client
            .request(method, &request.url)
            .timeout(std::time::Duration::from_millis(timeout));
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = &request.body {
            builder = builder.body(body.clone());
        }
        let response = builder
            .send()
            .await
            .map_err(|error| format!("network error requesting {}: {error}", request.url))?;
        let status = response.status().as_u16();
        let url = response.url().to_string();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_owned(), value.to_owned()))
            })
            .collect::<BTreeMap<_, _>>();
        let body = response
            .text()
            .await
            .map_err(|error| format!("network error reading {}: {error}", request.url))?;
        Ok(HttpResponse {
            status,
            headers,
            body,
            url,
        })
    }
}
