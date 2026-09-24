//! What hexa-infer's use cases need from the outside, as contracts.
//!
//! [`Endpoint`] is the registry's record of one backend — its URL, provider and status: the data
//! the registry adapter hands the router, not business domain, so it is a port-level type.

use async_trait::async_trait;
use hexa_core::ports::inference::{InferenceError, InferenceRequest, InferenceResponse};

/// Whatever serves a model. The use case builds a request and asks; which adapter answers is
/// wiring ([`crate::wiring`]).
#[async_trait]
pub trait Backends: Send + Sync {
    /// Run `request` on the backend that serves `request.model`.
    async fn complete(&self, request: InferenceRequest) -> Result<InferenceResponse, InferenceError>;

    /// Record what a completion cost.
    fn record_spend(&self, model: &str, input_tokens: u64, output_tokens: u64);
}
