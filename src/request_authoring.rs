use std::collections::HashMap;
use std::time::Duration;

use ulid::Ulid;

use crate::models::{AuthConfig, BodyConfig, HeaderField, HttpMethod, QueryParamField};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestTab {
    Body,
    Params,
    Headers,
    Auth,
    PostScript,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendButtonState {
    Disabled(SendDisabledReason),
    Ready,
    Sending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendDisabledReason {
    EmptyUrl,
    InvalidUrl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestAuthoringState {
    pub method: HttpMethod,
    pub url: String,
    pub active_tab: RequestTab,
    pub headers: Vec<HeaderField>,
    pub query_params: Vec<QueryParamField>,
    pub auth: AuthConfig,
    pub body: BodyConfig,
    pub post_script: Option<String>,
}

impl Default for RequestAuthoringState {
    fn default() -> Self {
        Self {
            method: HttpMethod::Get,
            url: String::new(),
            active_tab: RequestTab::Body,
            headers: vec![HeaderField {
                name: String::new(),
                value: String::new(),
                enabled: true,
                description: None,
                secret: false,
            }],
            query_params: vec![QueryParamField {
                name: String::new(),
                value: String::new(),
                enabled: true,
                description: None,
            }],
            auth: AuthConfig::None,
            body: BodyConfig::None,
            post_script: None,
        }
    }
}

impl RequestAuthoringState {
    pub fn send_button_state(&self) -> SendButtonState {
        let trimmed = self.url.trim();
        if trimmed.is_empty() {
            return SendButtonState::Disabled(SendDisabledReason::EmptyUrl);
        }
        if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
            return SendButtonState::Disabled(SendDisabledReason::InvalidUrl);
        }

        SendButtonState::Ready
    }

    pub fn set_header_value(&mut self, index: usize, name: String, value: String) {
        if let Some(header) = self.headers.get_mut(index) {
            header.name = name;
            header.value = value;
        }
        ensure_auto_append_header_row(&mut self.headers);
    }

    pub fn set_param_value(&mut self, index: usize, name: String, value: String) {
        if let Some(param) = self.query_params.get_mut(index) {
            param.name = name;
            param.value = value;
        }
        ensure_auto_append_param_row(&mut self.query_params);
    }
}

pub fn ensure_auto_append_header_row(headers: &mut Vec<HeaderField>) {
    if headers.is_empty() || headers.last().is_some_and(is_header_row_non_empty) {
        headers.push(HeaderField {
            name: String::new(),
            value: String::new(),
            enabled: true,
            description: None,
            secret: false,
        });
    }
}

pub fn ensure_auto_append_param_row(params: &mut Vec<QueryParamField>) {
    if params.is_empty() || params.last().is_some_and(is_param_row_non_empty) {
        params.push(QueryParamField {
            name: String::new(),
            value: String::new(),
            enabled: true,
            description: None,
        });
    }
}

fn is_header_row_non_empty(row: &HeaderField) -> bool {
    !(row.name.trim().is_empty() && row.value.trim().is_empty())
}

fn is_param_row_non_empty(row: &QueryParamField) -> bool {
    !(row.name.trim().is_empty() && row.value.trim().is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameValidationError {
    EmptyName,
}

pub fn validate_rename(current_name: &str, candidate_name: &str) -> Result<String, RenameValidationError> {
    let normalized = candidate_name.trim();
    if normalized.is_empty() {
        return Err(RenameValidationError::EmptyName);
    }
    if normalized.eq_ignore_ascii_case(current_name) {
        return Ok(normalized.to_string());
    }
    Ok(normalized.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveDraft {
    pub request_id: Ulid,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingSave {
    revision: u64,
    due_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebouncedSavePipeline {
    debounce: Duration,
    pending: HashMap<Ulid, PendingSave>,
}

impl DebouncedSavePipeline {
    pub fn new(debounce: Duration) -> Self {
        Self {
            debounce,
            pending: HashMap::new(),
        }
    }

    pub fn schedule(&mut self, request_id: Ulid, revision: u64, now_ms: u64) {
        let due_at_ms = now_ms.saturating_add(self.debounce.as_millis() as u64);
        self.pending.insert(
            request_id,
            PendingSave {
                revision,
                due_at_ms,
            },
        );
    }

    pub fn drain_due(&mut self, now_ms: u64) -> Vec<SaveDraft> {
        let due_ids: Vec<Ulid> = self
            .pending
            .iter()
            .filter_map(|(request_id, pending)| {
                (pending.due_at_ms <= now_ms).then_some(*request_id)
            })
            .collect();

        let mut due = Vec::with_capacity(due_ids.len());
        for request_id in due_ids {
            if let Some(pending) = self.pending.remove(&request_id) {
                due.push(SaveDraft {
                    request_id,
                    revision: pending.revision,
                });
            }
        }
        due.sort_by_key(|draft| draft.request_id.to_string());
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_button_state_requires_non_empty_valid_url() {
        let mut state = RequestAuthoringState::default();
        assert_eq!(
            state.send_button_state(),
            SendButtonState::Disabled(SendDisabledReason::EmptyUrl)
        );

        state.url = "example.com".to_string();
        assert_eq!(
            state.send_button_state(),
            SendButtonState::Disabled(SendDisabledReason::InvalidUrl)
        );

        state.url = "https://example.com".to_string();
        assert_eq!(state.send_button_state(), SendButtonState::Ready);
    }

    #[test]
    fn editing_last_header_or_param_auto_appends_empty_row() {
        let mut state = RequestAuthoringState::default();
        assert_eq!(state.headers.len(), 1);
        state.set_header_value(0, "X-Token".to_string(), "secret".to_string());
        assert_eq!(state.headers.len(), 2);
        assert!(state.headers[1].name.is_empty());
        assert!(state.headers[1].value.is_empty());

        assert_eq!(state.query_params.len(), 1);
        state.set_param_value(0, "page".to_string(), "1".to_string());
        assert_eq!(state.query_params.len(), 2);
        assert!(state.query_params[1].name.is_empty());
        assert!(state.query_params[1].value.is_empty());
    }

    #[test]
    fn rename_validation_enforces_non_empty_names() {
        assert_eq!(
            validate_rename("Get User", "  "),
            Err(RenameValidationError::EmptyName)
        );
        assert_eq!(
            validate_rename("Get User", "  Fetch User  "),
            Ok("Fetch User".to_string())
        );
        assert_eq!(
            validate_rename("Get User", "get user"),
            Ok("get user".to_string())
        );
    }

    #[test]
    fn debounced_save_pipeline_only_flushes_after_due_time() {
        let mut pipeline = DebouncedSavePipeline::new(Duration::from_millis(500));
        let request_id = Ulid::new();
        pipeline.schedule(request_id, 1, 1_000);
        assert!(pipeline.drain_due(1_300).is_empty());

        pipeline.schedule(request_id, 2, 1_400);
        assert!(pipeline.drain_due(1_850).is_empty());

        let due = pipeline.drain_due(1_900);
        assert_eq!(
            due,
            vec![SaveDraft {
                request_id,
                revision: 2
            }]
        );
    }
}
