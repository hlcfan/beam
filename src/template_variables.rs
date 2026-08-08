use std::collections::HashMap;

use chrono::{DateTime, SecondsFormat, Utc};
use uuid::Uuid;

const TIMESTAMP: &str = "$timestamp";
const ISO_TIMESTAMP: &str = "$isoTimestamp";
const GUID: &str = "$guid";
const RANDOM_UUID: &str = "$randomUUID";

pub struct DynamicVariableContext {
    timestamp: DateTime<Utc>,
}

impl DynamicVariableContext {
    pub fn new() -> Self {
        Self {
            timestamp: Utc::now(),
        }
    }

    pub fn resolve(&self, name: &str) -> Option<String> {
        match name {
            "$timestamp" => Some(self.timestamp.timestamp().to_string()),
            "$isoTimestamp" => Some(self.timestamp.to_rfc3339_opts(SecondsFormat::Millis, true)),
            "$guid" | "$randomUUID" => Some(Uuid::new_v4().to_string()),
            _ => None,
        }
    }
}

pub fn is_dynamic_variable(name: &str) -> bool {
    matches!(name, TIMESTAMP | ISO_TIMESTAMP | GUID | RANDOM_UUID)
}

pub fn contains_dynamic_variable(input: &str) -> bool {
    template_variable_names(input).any(is_dynamic_variable)
}

pub fn resolve_template_variables(
    input: &str,
    environment: &HashMap<String, String>,
    dynamic: &DynamicVariableContext,
) -> String {
    let mut output = String::new();
    let mut index = 0usize;

    while let Some(start_offset) = input[index..].find("{{") {
        let start = index + start_offset;
        output.push_str(&input[index..start]);
        let token_start = start + 2;
        let Some(end_offset) = input[token_start..].find("}}") else {
            output.push_str(&input[start..]);
            return output;
        };
        let end = token_start + end_offset;
        let variable_name = input[token_start..end].trim();
        if let Some(value) = environment.get(variable_name) {
            output.push_str(value);
        } else if let Some(value) = dynamic.resolve(variable_name) {
            output.push_str(&value);
        } else {
            output.push_str(&input[start..end + 2]);
        }
        index = end + 2;
    }

    output.push_str(&input[index..]);
    output
}

fn template_variable_names(input: &str) -> impl Iterator<Item = &str> {
    input.split("{{").skip(1).filter_map(|remainder| {
        let (name, _) = remainder.split_once("}}")?;
        Some(name.trim())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_common_dynamic_variables() {
        let context = DynamicVariableContext {
            timestamp: DateTime::parse_from_rfc3339("2020-06-09T21:10:36.177Z")
                .unwrap()
                .with_timezone(&Utc),
        };
        let resolved = resolve_template_variables(
            "{{$timestamp}} {{$isoTimestamp}} {{$guid}} {{$randomUUID}}",
            &HashMap::new(),
            &context,
        );
        let values = resolved.split_whitespace().collect::<Vec<_>>();

        assert_eq!(values[0], "1591737036");
        assert_eq!(values[1], "2020-06-09T21:10:36.177Z");
        assert!(Uuid::parse_str(values[2]).is_ok());
        assert!(Uuid::parse_str(values[3]).is_ok());
    }

    #[test]
    fn environment_values_take_precedence_over_dynamic_variables() {
        let environment = HashMap::from([(TIMESTAMP.to_string(), "fixed".to_string())]);
        let context = DynamicVariableContext::new();

        assert_eq!(
            resolve_template_variables("{{$timestamp}}", &environment, &context),
            "fixed"
        );
    }

    #[test]
    fn preserves_unknown_and_unclosed_variables() {
        let context = DynamicVariableContext::new();

        assert_eq!(
            resolve_template_variables(
                "{{unknown}} {{$Timestamp}} {{unclosed",
                &HashMap::new(),
                &context,
            ),
            "{{unknown}} {{$Timestamp}} {{unclosed"
        );
    }

    #[test]
    fn detects_only_complete_supported_dynamic_variables() {
        assert!(contains_dynamic_variable(
            "https://example.com/{{$timestamp}}"
        ));
        assert!(!contains_dynamic_variable(
            "https://example.com/{{$Timestamp}}"
        ));
        assert!(!contains_dynamic_variable(
            "https://example.com/{{$timestamp"
        ));
    }
}
