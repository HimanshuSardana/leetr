//! An [`HttpClient`] for GPUI backed by the same blocking `reqwest` client the
//! rest of the app uses.
//!
//! GPUI loads remote images (statement diagrams live on `assets.leetcode.com`)
//! through the application's [`HttpClient`], and its default does nothing but
//! return an error. GPUI's own `reqwest` client would work, but it is
//! asynchronous and drags in a second TLS stack (`rustls-platform-verifier`,
//! and `jni` for Android) that this project has no other use for.
//!
//! Leetr already avoids an async runtime on purpose: jobs run on plain threads
//! and post results back through a channel (see [`crate::bg`]). This module
//! keeps that shape — the request runs on a short-lived thread using the
//! blocking client, and the future GPUI awaits is just a channel read.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context as _, Result};
use futures::channel::oneshot;
use futures::future::BoxFuture;
use futures::FutureExt as _;
use gpui_kit::http_client::http::HeaderValue;
use gpui_kit::http_client::{AsyncBody, HttpClient, Inner, Request, Response};

/// User agent for image downloads. LeetCode's CDN serves the same bytes to any
/// client, but sending the app's real identity keeps its logs honest.
const USER_AGENT: &str = concat!("leetr/", env!("CARGO_PKG_VERSION"));

/// Ceiling on a single image, so one stray asset cannot balloon memory.
const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;

/// A blocking `reqwest` client presented to GPUI as an [`HttpClient`].
pub struct BlockingHttpClient {
    client: reqwest::blocking::Client,
}

impl BlockingHttpClient {
    /// Fetch a URL's bytes with the same client GPUI uses for `img(..)`.
    ///
    /// Exposed so the image path can be exercised without a window — the live
    /// tests use it. In the app, GPUI drives the client through the
    /// [`HttpClient`] impl instead.
    pub fn get_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let response = self
            .client
            .get(url)
            .send()
            .with_context(|| format!("requesting {url}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(anyhow!("{url} answered {status}"));
        }
        let bytes = response.bytes().context("reading the response body")?;
        Ok(bytes.to_vec())
    }

    /// Build a client suitable for fetching images.
    pub fn new() -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(USER_AGENT)
            .build()
            .context("building the image HTTP client")?;
        Ok(Self { client })
    }
}

impl HttpClient for BlockingHttpClient {
    fn user_agent(&self) -> Option<&HeaderValue> {
        None
    }

    fn proxy(&self) -> Option<&gpui_kit::http_client::Url> {
        None
    }

    fn send(&self, req: Request<AsyncBody>) -> BoxFuture<'static, Result<Response<AsyncBody>>> {
        let client = self.client.clone();
        let (tx, rx) = oneshot::channel();
        // The request itself blocks, so it gets its own thread; the future
        // GPUI polls is only waiting on the channel.
        std::thread::spawn(move || {
            let _ = tx.send(fetch(&client, req));
        });
        async move {
            rx.await
                .map_err(|_| anyhow!("the image request thread went away"))?
        }
        .boxed()
    }
}

/// Perform one request with the blocking client and read its body.
///
/// GPUI only ever issues bodies-less GETs here (image and asset fetches), so a
/// request carrying a body is rejected rather than silently dropped.
fn fetch(
    client: &reqwest::blocking::Client,
    req: Request<AsyncBody>,
) -> Result<Response<AsyncBody>> {
    let (parts, body) = req.into_parts();
    if !matches!(body.0, Inner::Empty) {
        return Err(anyhow!(
            "the blocking image client only supports empty request bodies"
        ));
    }

    let method = parts.method.clone();
    let mut request = client.request(method, parts.uri.to_string());
    for (name, value) in parts.headers.iter() {
        request = request.header(name, value);
    }

    let response = request
        .send()
        .with_context(|| format!("requesting {}", parts.uri))?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.bytes().context("reading the response body")?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(anyhow!(
            "response from {} is {} bytes, over the {MAX_IMAGE_BYTES} byte limit",
            parts.uri,
            bytes.len()
        ));
    }

    let mut builder = Response::builder().status(status);
    for (name, value) in headers.iter() {
        builder = builder.header(name, value);
    }
    builder
        .body(AsyncBody::from(bytes.to_vec()))
        .context("building the response")
}

/// The client to hand to GPUI, or `None` when it could not be built.
///
/// A failure here only costs remote images, so it must never stop the app.
pub fn image_client() -> Option<Arc<dyn HttpClient>> {
    match BlockingHttpClient::new() {
        Ok(client) => Some(Arc::new(client)),
        Err(err) => {
            eprintln!("warning: images will not load: {err:#}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_only_accepts_body_less_requests() {
        let client = BlockingHttpClient::new().expect("client");
        let request = Request::builder()
            .uri("https://example.invalid/x")
            .body(AsyncBody::from("payload"))
            .unwrap();
        let err = match fetch(&client.client, request) {
            Ok(_) => panic!("a request with a body must be rejected"),
            Err(err) => err.to_string(),
        };
        assert!(
            err.contains("empty request bodies"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn a_request_to_an_unresolvable_host_is_an_error_not_a_panic() {
        let client = BlockingHttpClient::new().expect("client");
        let request = Request::builder()
            .uri("https://example.invalid/x")
            .body(AsyncBody::empty())
            .unwrap();
        assert!(fetch(&client.client, request).is_err());
    }

    #[test]
    fn image_client_never_panics() {
        let _ = image_client();
    }
}
