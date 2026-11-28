//! The one HTTP client every provider is asked through, and how a request to a provider goes out
//! and its answer is read.

use std::{error::Error, fmt::Display, time::Duration};

use reqwest::{
    header::{HeaderMap, RETRY_AFTER},
    redirect, Client, Response, StatusCode,
};
use serde::Serialize;

use super::{rate_limit::RateLimit, Caller, MetadataProvider};
use crate::error_types::MetadataError;

/// Long enough for a slow provider, short enough that an admin isn't kept waiting on a dead one.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const TIMEOUT: Duration = Duration::from_secs(20);

/// Far above any real answer — AniList's ten candidates are some 100 kB — so a provider gone
/// wrong can't fill the server's memory.
pub const MAX_ANSWER_BYTES: usize = 2 * 1024 * 1024;

/// When a 429 names no wait in seconds: AniList counts its limit per minute.
const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(60);

/// The client every provider is asked through, giving up on a request after `timeout`.
pub fn client(timeout: Duration) -> reqwest::Result<Client> {
    Client::builder()
        // A redirect could lead the request, and what it carries, to a host other than the
        // provider's.
        .redirect(redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT.min(timeout))
        .timeout(timeout)
        .build()
}

/// One provider's address, the client it is reached with, and its pace.
#[derive(Debug, Clone)]
pub struct Endpoint {
    provider: MetadataProvider,
    url: String,
    client: Client,
    rate: RateLimit,
}

impl Endpoint {
    /// `provider` at `url`, its requests at least `interval` apart.
    pub fn new(provider: MetadataProvider, url: &str, client: Client, interval: Duration) -> Self {
        Endpoint {
            provider,
            url: url.to_string(),
            client,
            rate: RateLimit::new(interval),
        }
    }

    /// Posts `body` as JSON once the provider's turn comes, and answers the status and the body.
    /// A 429 moves the provider's next turn to the time it names and answers `RateLimited`; a
    /// request that fails or times out, or an answer over `MAX_ANSWER_BYTES`, `ProviderUnavailable`.
    pub async fn post_json(
        &self,
        body: &impl Serialize,
        caller: Caller,
    ) -> Result<(StatusCode, Vec<u8>), MetadataError> {
        self.rate
            .take_turn(caller.max_wait())
            .await
            .map_err(|retry_after| MetadataError::RateLimited { retry_after })?;
        let response = self
            .client
            .post(&self.url)
            .json(body)
            .send()
            .await
            .map_err(|err| self.failed(err))?;
        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            let retry_after = self.rate.defer(retry_after(response.headers()));
            tracing::warn!(
                "{} limits its requests: the next one waits {} s",
                self.provider.as_str(),
                retry_after.as_secs()
            );
            return Err(MetadataError::RateLimited { retry_after });
        }
        Ok((status, self.read(response).await?))
    }

    /// Logs why the provider can't be used now, and answers `ProviderUnavailable`.
    pub fn unavailable(&self, why: impl Display) -> MetadataError {
        tracing::warn!("{} is unavailable: {why}", self.provider.as_str());
        MetadataError::ProviderUnavailable
    }

    /// The answer's body, read until it ends or passes `MAX_ANSWER_BYTES`.
    async fn read(&self, mut response: Response) -> Result<Vec<u8>, MetadataError> {
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|err| self.failed(err))? {
            if body.len() + chunk.len() > MAX_ANSWER_BYTES {
                return Err(self.unavailable(format_args!(
                    "its answer is longer than {MAX_ANSWER_BYTES} bytes"
                )));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    fn failed(&self, err: reqwest::Error) -> MetadataError {
        // Without its URL: a later provider's key can travel in the query string.
        self.unavailable(causes(&err.without_url()))
    }
}

/// The wait a 429 names in whole seconds; `DEFAULT_RETRY_AFTER` when it names none, or a date.
fn retry_after(headers: &HeaderMap) -> Duration {
    headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse().ok())
        .map_or(DEFAULT_RETRY_AFTER, Duration::from_secs)
}

/// `err` and each error it came from, on one line: reqwest's own text alone doesn't say what went
/// wrong.
fn causes(err: &dyn Error) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    fn naming(value: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static(value));
        headers
    }

    #[test]
    fn a_429_is_waited_out_for_the_seconds_it_names_or_a_minute() {
        assert_eq!(retry_after(&naming("30")), Duration::from_secs(30));
        assert_eq!(retry_after(&naming(" 7 ")), Duration::from_secs(7));
        assert_eq!(retry_after(&naming("0")), Duration::ZERO);
        for unusable in ["Wed, 21 Oct 2026 07:28:00 GMT", "-5", "1.5", ""] {
            assert_eq!(
                retry_after(&naming(unusable)),
                DEFAULT_RETRY_AFTER,
                "{unusable:?}"
            );
        }
        assert_eq!(retry_after(&HeaderMap::new()), DEFAULT_RETRY_AFTER);
    }
}
