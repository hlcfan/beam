use crate::models::{BodyConfig, BodyFormatKind, QueryParamField};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum RequestBodyFormat {
    None,
    Json,
    Xml,
    Graphql,
    Text,
    FormUrlEncoded,
    Multipart,
}

pub(in crate::ui) enum BodyFormatHint<'a> {
    FromConfig(&'a BodyConfig),
    FromContentType(Option<&'a str>),
}

pub(in crate::ui) fn body_editor_text(body: &BodyConfig) -> String {
    match body {
        BodyConfig::None => String::new(),
        BodyConfig::Raw { text, .. } | BodyConfig::Json { text } | BodyConfig::Xml { text } => {
            text.clone()
        }
        BodyConfig::FormUrlEncoded { fields } | BodyConfig::Multipart { fields } => fields
            .iter()
            .map(|field| format!("{}={}", field.name, field.value))
            .collect::<Vec<_>>()
            .join("\n"),
        BodyConfig::Graphql {
            query,
            variables_json,
        } => match variables_json {
            Some(variables) if !variables.is_empty() => {
                format!("query:\n{query}\n\nvariables:\n{variables}")
            }
            _ => query.clone(),
        },
    }
}

pub(in crate::ui) fn body_format_label(format: RequestBodyFormat) -> &'static str {
    match format {
        RequestBodyFormat::None => "None",
        RequestBodyFormat::Json => "JSON",
        RequestBodyFormat::Xml => "XML",
        RequestBodyFormat::Graphql => "GraphQL",
        RequestBodyFormat::Text => "Text",
        RequestBodyFormat::FormUrlEncoded => "Form URL",
        RequestBodyFormat::Multipart => "Multipart",
    }
}

pub(in crate::ui) fn body_tab_label(format: RequestBodyFormat) -> &'static str {
    match format {
        RequestBodyFormat::None => "Body",
        _ => body_format_label(format),
    }
}

pub(in crate::ui) fn supported_body_formats() -> [RequestBodyFormat; 7] {
    [
        RequestBodyFormat::None,
        RequestBodyFormat::Json,
        RequestBodyFormat::Xml,
        RequestBodyFormat::Graphql,
        RequestBodyFormat::Text,
        RequestBodyFormat::FormUrlEncoded,
        RequestBodyFormat::Multipart,
    ]
}

pub(in crate::ui) fn body_format_from_config(body: &BodyConfig) -> RequestBodyFormat {
    match body {
        BodyConfig::None => RequestBodyFormat::None,
        BodyConfig::Raw { .. } => RequestBodyFormat::Text,
        BodyConfig::Json { .. } => RequestBodyFormat::Json,
        BodyConfig::Xml { .. } => RequestBodyFormat::Xml,
        BodyConfig::FormUrlEncoded { .. } => RequestBodyFormat::FormUrlEncoded,
        BodyConfig::Multipart { .. } => RequestBodyFormat::Multipart,
        BodyConfig::Graphql { .. } => RequestBodyFormat::Graphql,
    }
}

fn parse_form_body_fields(text: &str) -> Vec<QueryParamField> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (name, value) = line
                .split_once('=')
                .map(|(name, value)| (name.trim().to_string(), value.to_string()))
                .unwrap_or_else(|| (line.to_string(), String::new()));
            QueryParamField {
                name,
                value,
                enabled: true,
                description: None,
            }
        })
        .collect()
}

fn parse_graphql_editor_text(text: &str) -> (String, Option<String>) {
    if let Some(rest) = text.strip_prefix("query:\n") {
        if let Some((query, variables)) = rest.split_once("\n\nvariables:\n") {
            let variables = variables.trim().to_string();
            return (
                query.to_string(),
                (!variables.is_empty()).then_some(variables),
            );
        }
        return (rest.to_string(), None);
    }
    (text.to_string(), None)
}

pub(in crate::ui) fn body_with_updated_text(current: &BodyConfig, text: String) -> BodyConfig {
    match current {
        BodyConfig::None => BodyConfig::Raw {
            media_type: None,
            text,
        },
        BodyConfig::Raw { media_type, .. } => BodyConfig::Raw {
            media_type: media_type.clone(),
            text,
        },
        BodyConfig::Json { .. } => BodyConfig::Json { text },
        BodyConfig::Xml { .. } => BodyConfig::Xml { text },
        BodyConfig::FormUrlEncoded { .. } => BodyConfig::FormUrlEncoded {
            fields: parse_form_body_fields(&text),
        },
        BodyConfig::Multipart { .. } => BodyConfig::Multipart {
            fields: parse_form_body_fields(&text),
        },
        BodyConfig::Graphql { .. } => {
            let (query, variables_json) = parse_graphql_editor_text(&text);
            BodyConfig::Graphql {
                query,
                variables_json,
            }
        }
    }
}

pub(in crate::ui) fn body_from_format(format: RequestBodyFormat, text: String) -> BodyConfig {
    match format {
        RequestBodyFormat::None => BodyConfig::None,
        RequestBodyFormat::Json => BodyConfig::Json { text },
        RequestBodyFormat::Xml => BodyConfig::Xml { text },
        RequestBodyFormat::Graphql => {
            let (query, variables_json) = parse_graphql_editor_text(&text);
            BodyConfig::Graphql {
                query,
                variables_json,
            }
        }
        RequestBodyFormat::Text => BodyConfig::Raw {
            media_type: Some("text/plain".to_string()),
            text,
        },
        RequestBodyFormat::FormUrlEncoded => BodyConfig::FormUrlEncoded {
            fields: parse_form_body_fields(&text),
        },
        RequestBodyFormat::Multipart => BodyConfig::Multipart {
            fields: parse_form_body_fields(&text),
        },
    }
}

pub(in crate::ui) fn format_body_text(
    text: &str,
    hint: BodyFormatHint<'_>,
) -> Result<String, String> {
    match hint {
        BodyFormatHint::FromConfig(body) => format_body_text_from_config(text, body),
        BodyFormatHint::FromContentType(content_type) => {
            format_body_text_from_content_type(text, content_type)
        }
    }
}

fn format_body_text_by_kind(text: &str, kind: BodyFormatKind) -> Result<String, String> {
    match kind {
        BodyFormatKind::Json => {
            let value = serde_json::from_str::<serde_json::Value>(text)
                .map_err(|err| format!("Unable to format JSON body: {err}"))?;
            serde_json::to_string_pretty(&value)
                .map_err(|err| format!("Unable to format JSON body: {err}"))
        }
        BodyFormatKind::Xml => {
            format_xml_or_html(text).ok_or_else(|| "Unable to format XML/HTML body.".to_string())
        }
        BodyFormatKind::Graphql => {
            let (query, variables_json) = parse_graphql_editor_text(text);
            let query = query.trim().to_string();
            let formatted_variables = if let Some(variables) = variables_json {
                let trimmed = variables.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    let value = serde_json::from_str::<serde_json::Value>(trimmed)
                        .map_err(|err| format!("Unable to format GraphQL variables JSON: {err}"))?;
                    Some(
                        serde_json::to_string_pretty(&value).map_err(|err| {
                            format!("Unable to format GraphQL variables JSON: {err}")
                        })?,
                    )
                }
            } else {
                None
            };

            if let Some(variables) = formatted_variables {
                Ok(format!("query:\n{query}\n\nvariables:\n{variables}"))
            } else {
                Ok(query)
            }
        }
        BodyFormatKind::Form => Ok(parse_form_body_fields(text)
            .into_iter()
            .map(|field| format!("{}={}", field.name.trim(), field.value.trim()))
            .collect::<Vec<_>>()
            .join("\n")),
    }
}

fn format_body_text_from_config(text: &str, body_config: &BodyConfig) -> Result<String, String> {
    let kind = match body_config {
        BodyConfig::Json { .. } => BodyFormatKind::Json,
        BodyConfig::Xml { .. } => BodyFormatKind::Xml,
        BodyConfig::Graphql { .. } => BodyFormatKind::Graphql,
        BodyConfig::FormUrlEncoded { .. } | BodyConfig::Multipart { .. } => BodyFormatKind::Form,
        _ => {
            return Err(
                "Formatting is only supported for JSON, XML, GraphQL, and form bodies.".into(),
            );
        }
    };
    format_body_text_by_kind(text, kind)
}

fn format_body_text_from_content_type(
    body: &str,
    content_type: Option<&str>,
) -> Result<String, String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err("Body is empty.".into());
    }

    let ct = content_type.unwrap_or("").to_lowercase();
    let kind = if ct.contains("json")
        || ct.contains("graphql")
        || trimmed.starts_with('{')
        || trimmed.starts_with('[')
    {
        if ct.contains("graphql") {
            BodyFormatKind::Graphql
        } else {
            BodyFormatKind::Json
        }
    } else if ct.contains("xml") || ct.contains("html") {
        BodyFormatKind::Xml
    } else if ct.contains("x-www-form-urlencoded") || ct.contains("multipart") {
        BodyFormatKind::Form
    } else if trimmed.starts_with("query:") {
        BodyFormatKind::Graphql
    } else {
        return Err("Unable to format body for the detected content type.".into());
    };

    format_body_text_by_kind(trimmed, kind)
}

fn format_xml_or_html(text: &str) -> Option<String> {
    let mut result = String::with_capacity(text.len() * 2);
    let mut depth = 0usize;
    let mut i = 0usize;
    let bytes = text.as_bytes();

    while i < bytes.len() {
        if bytes[i] == b'<' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'>' {
                j += 1;
            }
            if j >= bytes.len() {
                let remainder = text[i..].trim();
                if !remainder.is_empty() {
                    if !result.is_empty() && !result.ends_with('\n') {
                        result.push('\n');
                        for _ in 0..depth {
                            result.push_str("  ");
                        }
                    }
                    result.push_str(remainder);
                }
                break;
            }
            let tag = text[i..=j].trim();
            let is_closing = tag.starts_with("</");
            let is_self_closing = tag.ends_with("/>");
            let is_comment = tag.starts_with("<!--")
                || tag.starts_with("<?")
                || tag.starts_with("<!DOCTYPE")
                || tag.starts_with("<![CDATA[");

            if is_closing {
                depth = depth.saturating_sub(1);
            }
            if !result.is_empty() {
                result.push('\n');
            }
            for _ in 0..depth {
                result.push_str("  ");
            }
            result.push_str(tag);

            let mut k = j + 1;
            while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                k += 1;
            }
            let next_is_tag = k < bytes.len() && bytes[k] == b'<';
            let next_is_closing = next_is_tag && k + 1 < bytes.len() && bytes[k + 1] == b'/';

            if !is_closing && !is_self_closing && !is_comment && !(next_is_tag && next_is_closing) {
                depth += 1;
            }
            i = j + 1;
        } else if !bytes[i].is_ascii_whitespace() {
            let start = i;
            while i < bytes.len() && bytes[i] != b'<' {
                i += 1;
            }
            let text_content = text[start..i].trim();
            if !text_content.is_empty() {
                if !result.is_empty() && !result.ends_with('\n') {
                    result.push('\n');
                    for _ in 0..depth {
                        result.push_str("  ");
                    }
                }
                result.push_str(text_content);
            }
        } else {
            i += 1;
        }
    }

    (!result.is_empty()).then_some(result)
}

pub(in crate::ui) fn body_editor_language(body_config: &BodyConfig) -> &'static str {
    match body_config {
        BodyConfig::Json { .. } => "json",
        BodyConfig::Xml { .. } => "html",
        BodyConfig::Graphql { .. } => "graphql",
        _ => "text",
    }
}

pub(in crate::ui) fn response_body_editor_language(content_type: Option<&str>) -> &'static str {
    let ct = content_type.unwrap_or("").to_lowercase();
    if ct.contains("graphql") {
        "graphql"
    } else if ct.contains("json") {
        "json"
    } else if ct.contains("xml") || ct.contains("html") {
        "html"
    } else {
        "text"
    }
}

#[cfg(test)]
mod tests {
    use super::{BodyFormatHint, format_body_text};
    use crate::models::BodyConfig;

    fn from_config(input: &str, body: &BodyConfig) -> Result<String, String> {
        format_body_text(input, BodyFormatHint::FromConfig(body))
    }

    fn from_content_type(input: &str, content_type: Option<&str>) -> Result<String, String> {
        format_body_text(input, BodyFormatHint::FromContentType(content_type))
    }

    #[test]
    fn format_body_text_json() {
        let result = from_config(
            r#"{"name": "John", "age": 30}"#,
            &BodyConfig::Json {
                text: String::new(),
            },
        );
        assert_eq!(
            result.unwrap(),
            "{\n  \"name\": \"John\",\n  \"age\": 30\n}"
        );
    }

    #[test]
    fn format_body_text_json_array() {
        let result = from_content_type(r#"[{"id": 1}, {"id": 2}]"#, Some("application/json"));
        assert!(result.unwrap().contains("\"id\": 1"));
    }

    #[test]
    fn format_body_text_json_invalid() {
        let error = from_config(
            "not json",
            &BodyConfig::Json {
                text: String::new(),
            },
        )
        .unwrap_err();
        assert!(error.contains("Unable to format JSON body"));
    }

    #[test]
    fn format_body_text_xml() {
        let formatted = from_config(
            "<root><name>John</name></root>",
            &BodyConfig::Xml {
                text: String::new(),
            },
        )
        .unwrap();
        assert!(formatted.starts_with("<root>"));
        assert!(formatted.contains("  <name>"));
        assert!(formatted.contains("    John"));
        assert!(formatted.contains("  </name>"));
        assert!(formatted.ends_with("</root>"));
    }

    #[test]
    fn format_body_text_xml_non_xml_passes_through() {
        let result = from_config(
            "not xml",
            &BodyConfig::Xml {
                text: String::new(),
            },
        );
        assert_eq!(result.unwrap(), "not xml");
    }

    #[test]
    fn format_body_text_graphql_query_only() {
        let result = from_config(
            "query { user { name } }",
            &BodyConfig::Graphql {
                query: String::new(),
                variables_json: None,
            },
        );
        assert_eq!(result.unwrap(), "query { user { name } }");
    }

    #[test]
    fn format_body_text_graphql_with_variables() {
        let result = from_config(
            "query:\nquery { user(id: $id) { name } }\n\nvariables:\n{\"id\": 1}",
            &BodyConfig::Graphql {
                query: String::new(),
                variables_json: None,
            },
        )
        .unwrap();
        assert!(result.contains("query { user(id: $id) { name } }"));
        assert!(result.contains("\"id\": 1"));
    }

    #[test]
    fn format_body_text_graphql_with_empty_variables() {
        let result = from_config(
            "query:\nquery { user { name } }\n\nvariables:\n",
            &BodyConfig::Graphql {
                query: String::new(),
                variables_json: None,
            },
        );
        assert_eq!(result.unwrap(), "query { user { name } }");
    }

    #[test]
    fn format_body_text_graphql_invalid_variables_json() {
        let error = from_config(
            "query:\nquery { user { name } }\n\nvariables:\n{invalid}",
            &BodyConfig::Graphql {
                query: String::new(),
                variables_json: None,
            },
        )
        .unwrap_err();
        assert!(error.contains("Unable to format GraphQL variables JSON"));
    }

    #[test]
    fn format_body_text_form() {
        let result = from_config(
            "key1=value1\nkey2=value2",
            &BodyConfig::FormUrlEncoded { fields: vec![] },
        );
        assert_eq!(result.unwrap(), "key1=value1\nkey2=value2");
    }

    #[test]
    fn format_body_text_form_whitespace_trimming() {
        let result = from_config(
            "  key1 = value1  \n  key2 = value2  ",
            &BodyConfig::FormUrlEncoded { fields: vec![] },
        );
        assert_eq!(result.unwrap(), "key1=value1\nkey2=value2");
    }

    #[test]
    fn format_body_text_form_empty_lines_skipped() {
        let result = from_config(
            "key1=value1\n\n\nkey2=value2",
            &BodyConfig::FormUrlEncoded { fields: vec![] },
        );
        assert_eq!(result.unwrap(), "key1=value1\nkey2=value2");
    }

    #[test]
    fn format_body_text_form_no_equals_sign() {
        let result = from_config(
            "key_without_value",
            &BodyConfig::FormUrlEncoded { fields: vec![] },
        );
        assert_eq!(result.unwrap(), "key_without_value=");
    }

    #[test]
    fn format_body_text_from_config_multipart() {
        let result = from_config(
            "key1=value1\nkey2=value2",
            &BodyConfig::Multipart { fields: vec![] },
        );
        assert_eq!(result.unwrap(), "key1=value1\nkey2=value2");
    }

    #[test]
    fn format_body_text_from_config_unsupported_body_type() {
        let error = from_config("some text", &BodyConfig::None).unwrap_err();
        assert!(
            error.contains("Formatting is only supported for JSON, XML, GraphQL, and form bodies.")
        );
    }

    #[test]
    fn format_body_text_from_config_raw_body_type() {
        let error = from_config(
            "some text",
            &BodyConfig::Raw {
                media_type: None,
                text: String::new(),
            },
        )
        .unwrap_err();
        assert!(
            error.contains("Formatting is only supported for JSON, XML, GraphQL, and form bodies.")
        );
    }

    #[test]
    fn format_body_text_from_content_type_empty_body() {
        let error = from_content_type("  ", Some("application/json")).unwrap_err();
        assert!(error.contains("Body is empty"));
    }

    #[test]
    fn format_body_text_from_content_type_unrecognized() {
        let error = from_content_type("some text", Some("application/pdf")).unwrap_err();
        assert!(error.contains("Unable to format body for the detected content type"));
    }

    #[test]
    fn format_body_text_from_content_type_auto_detect_json_brace() {
        let formatted = from_content_type(r#"{"key": "value"}"#, None).unwrap();
        assert!(formatted.contains("\"key\": \"value\""));
    }

    #[test]
    fn format_body_text_from_content_type_auto_detect_json_bracket() {
        assert!(from_content_type("[1, 2, 3]", None).is_ok());
    }

    #[test]
    fn format_body_text_from_content_type_graphql_content_type() {
        let result =
            from_content_type("query { user { name } }", Some("application/graphql")).unwrap();
        assert_eq!(result, "query { user { name } }");
    }

    #[test]
    fn format_body_text_from_content_type_graphql_query_prefix() {
        let result = from_content_type("query:\nquery { user { name } }", None).unwrap();
        assert!(result.contains("query { user { name } }"));
    }

    #[test]
    fn format_body_text_from_content_type_html() {
        let formatted =
            from_content_type("<html><body><p>Hello</p></body></html>", Some("text/html")).unwrap();
        for fragment in ["<html>", "<p>", "Hello", "</p>", "</body>", "</html>"] {
            assert!(formatted.contains(fragment));
        }
    }

    #[test]
    fn format_body_text_json_preserves_insertion_order() {
        let result = from_config(
            r#"{"z": 1, "a": 2, "m": 3}"#,
            &BodyConfig::Json {
                text: String::new(),
            },
        );
        assert_eq!(
            result.unwrap(),
            "{\n  \"z\": 1,\n  \"a\": 2,\n  \"m\": 3\n}"
        );
    }

    #[test]
    fn format_body_text_json_nested() {
        let result = from_config(
            r#"{"outer": {"inner": [1, 2, 3]}}"#,
            &BodyConfig::Json {
                text: String::new(),
            },
        )
        .unwrap();
        assert!(result.contains("\"outer\": {"));
        assert!(result.contains("\"inner\": ["));
    }

    #[test]
    fn format_body_text_from_content_type_auto_xml() {
        let formatted = from_content_type("<note><to>Tove</to></note>", Some("text/xml")).unwrap();
        assert!(formatted.starts_with("<note>"));
        assert!(formatted.contains("  <to>"));
        assert!(formatted.contains("    Tove"));
        assert!(formatted.contains("  </to>"));
        assert!(formatted.ends_with("</note>"));
    }
}
