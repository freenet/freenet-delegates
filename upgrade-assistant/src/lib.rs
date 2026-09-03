//! Upgrade Assistant Delegate
//!
//! A general-purpose delegate that helps ANY delegate upgrade gracefully.
//! This delegate is designed to be extremely simple and stable, rarely (or never) changing.
//!
//! ## Purpose
//!
//! When a delegate's WASM code changes, its delegate key changes (derived from code hash).
//! This creates a new, empty delegate storage, making data from the previous version
//! inaccessible.
//!
//! The Upgrade Assistant solves this by storing delegate key mappings:
//! - Maps namespace strings to their current delegate key
//! - Each UI contract has its own isolated namespace (partitioned by attested origin)
//!
//! ## Security Model
//!
//! Storage is partitioned by the **attested origin** (contract key of the requesting UI).
//! This means:
//! - Different UIs cannot see or modify each other's data
//! - The origin is cryptographically verified by Freenet and cannot be spoofed
//!
//! ## Usage
//!
//! 1. On startup, a delegate queries the Upgrade Assistant for its previous key
//! 2. If the stored key differs from the current key, the delegate migrates data
//! 3. The delegate updates the Upgrade Assistant with its new current key
//!
//! ## API
//!
//! See [`UpgradeAssistantRequest`] and [`UpgradeAssistantResponse`] for the message types.

#![allow(unexpected_cfgs)]

mod previous_versions;

use freenet_stdlib::prelude::{
    delegate, ApplicationMessage, DelegateContext, DelegateError, DelegateInterface,
    GetSecretRequest, GetSecretResponse, InboundDelegateMsg, OutboundDelegateMsg, Parameters,
    SecretsId, SetSecretRequest,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use previous_versions::{PreviousVersion, PREVIOUS_UPGRADE_ASSISTANT_KEYS};

// Re-export types for external use
pub mod types {
    pub use super::{UpgradeAssistantRequest, UpgradeAssistantResponse};
}

/// Messages the Upgrade Assistant accepts
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum UpgradeAssistantRequest {
    /// Get the stored delegate key for a namespace.
    /// Namespace is optional - use None if app has only one delegate.
    GetPreviousKey { namespace: Option<String> },

    /// Store/update the delegate key for a namespace.
    SetCurrentKey {
        namespace: Option<String>,
        delegate_key: [u8; 32],
        code_hash: [u8; 32],
    },
}

/// Responses from the Upgrade Assistant
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum UpgradeAssistantResponse {
    /// Response to GetPreviousKey
    PreviousKey {
        namespace: Option<String>,
        /// None if this namespace has never registered
        delegate_key: Option<[u8; 32]>,
        code_hash: Option<[u8; 32]>,
    },

    /// Response to SetCurrentKey
    KeyUpdated { namespace: Option<String> },
}

/// Origin contract that's making requests (attested by Freenet)
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct Origin(Vec<u8>);

/// Stored data for a delegate key mapping
#[derive(Serialize, Deserialize, Debug, Clone)]
struct StoredKeyInfo {
    delegate_key: [u8; 32],
    code_hash: [u8; 32],
}

/// Pending operation context
#[derive(Serialize, Deserialize, Debug, Clone)]
enum PendingOperation {
    GetPreviousKey {
        origin: Origin,
        namespace: Option<String>,
        app: [u8; 32], // Store app as bytes for serialization
    },
    SetCurrentKey {
        origin: Origin,
        namespace: Option<String>,
        delegate_key: [u8; 32],
        code_hash: [u8; 32],
    },
}

/// Context passed between delegate calls
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct UpgradeAssistantContext {
    pending_ops: HashMap<String, PendingOperation>,
}

impl TryFrom<DelegateContext> for UpgradeAssistantContext {
    type Error = DelegateError;

    fn try_from(ctx: DelegateContext) -> Result<Self, Self::Error> {
        let bytes = ctx.as_ref();
        if bytes.is_empty() {
            return Ok(Self::default());
        }
        ciborium::from_reader(bytes)
            .map_err(|e| DelegateError::Deser(format!("Failed to deserialize context: {e}")))
    }
}

impl TryFrom<&UpgradeAssistantContext> for DelegateContext {
    type Error = DelegateError;

    fn try_from(ctx: &UpgradeAssistantContext) -> Result<Self, Self::Error> {
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(ctx, &mut bytes)
            .map_err(|e| DelegateError::Deser(format!("Failed to serialize context: {e}")))?;
        Ok(DelegateContext::new(bytes))
    }
}

/// Create a unique secret ID for storing a key mapping.
/// Format: "upgrade_assistant:{origin_base58}:{namespace}"
fn create_storage_key(origin: &Origin, namespace: &Option<String>) -> SecretsId {
    let origin_b58 = bs58::encode(&origin.0).into_string();
    let ns = namespace.as_deref().unwrap_or("_default_");
    let key = format!("upgrade_assistant:{origin_b58}:{ns}");
    SecretsId::new(key.into_bytes())
}

/// Create a response message to send back to the application
fn create_app_response(
    response: &UpgradeAssistantResponse,
    context: &DelegateContext,
    app: freenet_stdlib::prelude::ContractInstanceId,
) -> Result<OutboundDelegateMsg, DelegateError> {
    let mut payload = Vec::new();
    ciborium::ser::into_writer(response, &mut payload)
        .map_err(|e| DelegateError::Deser(format!("Failed to serialize response: {e}")))?;

    Ok(OutboundDelegateMsg::ApplicationMessage(
        ApplicationMessage::new(app, payload).with_context(context.clone()),
    ))
}

/// Upgrade Assistant Delegate
///
/// A simple, stable delegate that helps other delegates upgrade gracefully
/// by tracking delegate key mappings partitioned by origin.
pub struct UpgradeAssistant;

#[delegate]
impl DelegateInterface for UpgradeAssistant {
    fn process(
        _parameters: Parameters<'static>,
        attested: Option<&'static [u8]>,
        message: InboundDelegateMsg,
    ) -> Result<Vec<OutboundDelegateMsg>, DelegateError> {
        // Verify that attested origin is provided
        let origin = match attested {
            Some(bytes) => Origin(bytes.to_vec()),
            None => {
                return Err(DelegateError::Other("missing attested origin".to_string()));
            }
        };

        match message {
            InboundDelegateMsg::ApplicationMessage(app_msg) => {
                if app_msg.processed {
                    return Err(DelegateError::Other(
                        "cannot process an already processed message".into(),
                    ));
                }
                handle_application_message(app_msg, &origin)
            }
            InboundDelegateMsg::GetSecretResponse(response) => handle_get_secret_response(response),
            InboundDelegateMsg::UserResponse(_) => Err(DelegateError::Other(
                "unexpected message type: UserResponse".into(),
            )),
            InboundDelegateMsg::GetSecretRequest(_) => Err(DelegateError::Other(
                "unexpected message type: GetSecretRequest".into(),
            )),
        }
    }
}

fn handle_application_message(
    app_msg: ApplicationMessage,
    origin: &Origin,
) -> Result<Vec<OutboundDelegateMsg>, DelegateError> {
    let mut context = UpgradeAssistantContext::try_from(app_msg.context)?;

    let request: UpgradeAssistantRequest = ciborium::from_reader(app_msg.payload.as_slice())
        .map_err(|e| DelegateError::Deser(format!("Failed to deserialize request: {e}")))?;

    match request {
        UpgradeAssistantRequest::GetPreviousKey { namespace } => {
            handle_get_previous_key(&mut context, origin, namespace, app_msg.app)
        }
        UpgradeAssistantRequest::SetCurrentKey {
            namespace,
            delegate_key,
            code_hash,
        } => handle_set_current_key(
            &mut context,
            origin,
            namespace,
            delegate_key,
            code_hash,
            app_msg.app,
        ),
    }
}

fn handle_get_previous_key(
    context: &mut UpgradeAssistantContext,
    origin: &Origin,
    namespace: Option<String>,
    app: freenet_stdlib::prelude::ContractInstanceId,
) -> Result<Vec<OutboundDelegateMsg>, DelegateError> {
    // Create the storage key for this origin + namespace
    let secret_id = create_storage_key(origin, &namespace);
    let secret_key = String::from_utf8_lossy(secret_id.key()).to_string();

    // Extract app bytes for storage in pending operation
    let app_bytes: [u8; 32] = (*app).into();

    // Store the pending operation
    context.pending_ops.insert(
        secret_key,
        PendingOperation::GetPreviousKey {
            origin: origin.clone(),
            namespace: namespace.clone(),
            app: app_bytes,
        },
    );

    // Serialize context (need immutable reference for TryFrom)
    let context_bytes = DelegateContext::try_from(&*context)?;

    // Request the stored key info
    let get_secret = OutboundDelegateMsg::GetSecretRequest(GetSecretRequest {
        key: secret_id,
        context: context_bytes,
        processed: false,
    });

    Ok(vec![get_secret])
}

fn handle_set_current_key(
    context: &mut UpgradeAssistantContext,
    origin: &Origin,
    namespace: Option<String>,
    delegate_key: [u8; 32],
    code_hash: [u8; 32],
    app: freenet_stdlib::prelude::ContractInstanceId,
) -> Result<Vec<OutboundDelegateMsg>, DelegateError> {
    // Create the storage key for this origin + namespace
    let secret_id = create_storage_key(origin, &namespace);

    // Create the stored key info
    let key_info = StoredKeyInfo {
        delegate_key,
        code_hash,
    };

    // Serialize the key info
    let mut value = Vec::new();
    ciborium::ser::into_writer(&key_info, &mut value)
        .map_err(|e| DelegateError::Deser(format!("Failed to serialize key info: {e}")))?;

    // Create response for the client
    let response = UpgradeAssistantResponse::KeyUpdated {
        namespace: namespace.clone(),
    };

    // Serialize context
    let context_bytes = DelegateContext::try_from(&*context)?;

    // Create the response message
    let app_response = create_app_response(&response, &context_bytes, app)?;

    // Store the key info
    let set_secret = OutboundDelegateMsg::SetSecretRequest(SetSecretRequest {
        key: secret_id,
        value: Some(value),
    });

    Ok(vec![app_response, set_secret])
}

fn handle_get_secret_response(
    response: GetSecretResponse,
) -> Result<Vec<OutboundDelegateMsg>, DelegateError> {
    let mut context = UpgradeAssistantContext::try_from(response.context.clone())?;

    let key_str = String::from_utf8_lossy(response.key.key()).to_string();

    // Find the pending operation
    let pending_op = context
        .pending_ops
        .remove(&key_str)
        .ok_or_else(|| DelegateError::Other(format!("No pending operation for key: {key_str}")))?;

    match pending_op {
        PendingOperation::GetPreviousKey { namespace, app, .. } => {
            // Parse the stored key info if present
            let (delegate_key, code_hash) = if let Some(value) = response.value {
                let key_info: StoredKeyInfo =
                    ciborium::from_reader(value.as_slice()).map_err(|e| {
                        DelegateError::Deser(format!("Failed to deserialize key info: {e}"))
                    })?;
                (Some(key_info.delegate_key), Some(key_info.code_hash))
            } else {
                (None, None)
            };

            // Create response
            let response = UpgradeAssistantResponse::PreviousKey {
                namespace,
                delegate_key,
                code_hash,
            };

            // Serialize context
            let context_bytes = DelegateContext::try_from(&context)?;

            // Reconstruct app from stored bytes
            let app = freenet_stdlib::prelude::ContractInstanceId::new(app);

            let app_response = create_app_response(&response, &context_bytes, app)?;

            Ok(vec![app_response])
        }
        PendingOperation::SetCurrentKey { .. } => {
            // This shouldn't happen - SetCurrentKey doesn't need a get response
            Err(DelegateError::Other(
                "Unexpected SetCurrentKey pending operation for get secret response".into(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freenet_stdlib::prelude::ContractInstanceId;

    fn create_test_parameters() -> Parameters<'static> {
        Parameters::from(vec![])
    }

    fn create_test_origin() -> &'static [u8] {
        static TEST_ORIGIN: [u8; 32] = [1u8; 32];
        &TEST_ORIGIN
    }

    fn create_app_message(
        request: UpgradeAssistantRequest,
        app_id: ContractInstanceId,
    ) -> ApplicationMessage {
        let mut payload = Vec::new();
        ciborium::ser::into_writer(&request, &mut payload).unwrap();
        ApplicationMessage::new(app_id, payload)
    }

    fn extract_response(messages: Vec<OutboundDelegateMsg>) -> Option<UpgradeAssistantResponse> {
        for msg in messages {
            if let OutboundDelegateMsg::ApplicationMessage(app_msg) = msg {
                return ciborium::from_reader(app_msg.payload.as_slice()).ok();
            }
        }
        None
    }

    #[test]
    fn test_set_current_key() {
        let delegate_key = [42u8; 32];
        let code_hash = [123u8; 32];

        let request = UpgradeAssistantRequest::SetCurrentKey {
            namespace: Some("test-delegate".to_string()),
            delegate_key,
            code_hash,
        };

        let app_id = ContractInstanceId::new([1u8; 32]);
        let app_msg = create_app_message(request, app_id);
        let inbound_msg = InboundDelegateMsg::ApplicationMessage(app_msg);

        let result = UpgradeAssistant::process(
            create_test_parameters(),
            Some(create_test_origin()),
            inbound_msg,
        )
        .unwrap();

        // Should have 2 messages: app response and set secret
        assert_eq!(result.len(), 2);

        // Check app response
        let response = extract_response(result.clone()).unwrap();
        match response {
            UpgradeAssistantResponse::KeyUpdated { namespace } => {
                assert_eq!(namespace, Some("test-delegate".to_string()));
            }
            _ => panic!("Expected KeyUpdated, got {:?}", response),
        }

        // Check set secret request
        let mut found_set_request = false;
        for msg in result {
            if let OutboundDelegateMsg::SetSecretRequest(req) = msg {
                assert!(req.value.is_some());
                found_set_request = true;
            }
        }
        assert!(found_set_request, "No SetSecretRequest found");
    }

    #[test]
    fn test_get_previous_key_request() {
        let request = UpgradeAssistantRequest::GetPreviousKey {
            namespace: Some("test-delegate".to_string()),
        };

        let app_id = ContractInstanceId::new([1u8; 32]);
        let app_msg = create_app_message(request, app_id);
        let inbound_msg = InboundDelegateMsg::ApplicationMessage(app_msg);

        let result = UpgradeAssistant::process(
            create_test_parameters(),
            Some(create_test_origin()),
            inbound_msg,
        )
        .unwrap();

        // Should have 1 message: get secret request
        assert_eq!(result.len(), 1);

        match &result[0] {
            OutboundDelegateMsg::GetSecretRequest(req) => {
                let key_str = String::from_utf8(req.key.key().to_vec()).unwrap();
                assert!(key_str.contains("upgrade_assistant"));
                assert!(key_str.contains("test-delegate"));
            }
            _ => panic!("Expected GetSecretRequest, got {:?}", result[0]),
        }
    }

    #[test]
    fn test_error_on_missing_attested() {
        let request = UpgradeAssistantRequest::GetPreviousKey { namespace: None };
        let app_id = ContractInstanceId::new([1u8; 32]);
        let app_msg = create_app_message(request, app_id);
        let inbound_msg = InboundDelegateMsg::ApplicationMessage(app_msg);

        let result = UpgradeAssistant::process(create_test_parameters(), None, inbound_msg);
        assert!(result.is_err());

        if let Err(DelegateError::Other(msg)) = result {
            assert!(msg.contains("missing attested origin"));
        } else {
            panic!("Expected DelegateError::Other");
        }
    }

    #[test]
    fn test_error_on_processed_message() {
        let request = UpgradeAssistantRequest::GetPreviousKey { namespace: None };
        let app_id = ContractInstanceId::new([1u8; 32]);
        let mut app_msg = create_app_message(request, app_id);
        app_msg = app_msg.processed(true);
        let inbound_msg = InboundDelegateMsg::ApplicationMessage(app_msg);

        let result = UpgradeAssistant::process(
            create_test_parameters(),
            Some(create_test_origin()),
            inbound_msg,
        );
        assert!(result.is_err());

        if let Err(DelegateError::Other(msg)) = result {
            assert!(msg.contains("cannot process an already processed message"));
        } else {
            panic!("Expected DelegateError::Other");
        }
    }
}
