use crate::error::BeamError;
use crate::models::{AuthConfig, BodyConfig, HeaderField, HttpMethod, QueryParamField};

/// Parsed cURL command in the request-authoring shape. This is a distinct
/// intermediate representation — it imports into the **currently open request**
/// in place rather than into a new workspace, so it stays separate from `ImportPlan`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurlPlan {
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<HeaderField>,
    pub query: Vec<QueryParamField>,
    pub body: BodyConfig,
    pub auth: AuthConfig,
}

pub fn is_curl(value: &str) -> bool {
    let trimmed = value.trim();
    let first_word = trimmed.split_whitespace().next().unwrap_or("");
    first_word.eq_ignore_ascii_case("curl")
}

pub fn parse(value: &str) -> Result<CurlPlan, BeamError> {
    let tokens = tokenize(value);
    if tokens.is_empty() {
        return Err(BeamError::Validation {
            message: "empty cURL command".to_string(),
        });
    }
    if !tokens[0].eq_ignore_ascii_case("curl") {
        return Err(BeamError::Validation {
            message: "not a cURL command".to_string(),
        });
    }

    let mut method: Option<HttpMethod> = None;
    let mut url: Option<String> = None;
    let mut headers: Vec<HeaderField> = Vec::new();
    let mut body_text: Option<String> = None;
    let mut content_type: Option<String> = None;
    let mut has_data = false;
    let mut form_fields: Vec<QueryParamField> = Vec::new();
    let mut urlencoded_fields: Vec<QueryParamField> = Vec::new();
    let mut auth: AuthConfig = AuthConfig::None;
    let mut cookie: Option<String> = None;

    let mut i = 1;
    while i < tokens.len() {
        let tok = &tokens[i];

        if !tok.starts_with('-') {
            if url.is_none() {
                url = Some(tok.clone());
            }
            i += 1;
            continue;
        }

        let (flag, inline) = split_flag(tok);
        let value: Option<String> = if let Some(v) = inline {
            Some(v.to_string())
        } else if takes_value(flag) {
            if i + 1 < tokens.len() {
                i += 1;
                Some(tokens[i].clone())
            } else {
                None
            }
        } else {
            None
        };

        match flag {
            "-X" | "--request" => {
                if let Some(ref m) = value {
                    method = parse_method(m);
                }
            }
            "-H" | "--header" => {
                if let Some(h) = value
                    && let Some((name, val)) = h.split_once(':')
                {
                    let name = name.trim().to_string();
                    let val = val.trim().to_string();
                    if name.is_empty() {
                        // malformed header — drop silently
                    } else if name.eq_ignore_ascii_case("content-type") {
                        content_type = Some(val.clone());
                        push_header(&mut headers, name, val);
                    } else if name.eq_ignore_ascii_case("cookie") {
                        cookie = Some(val);
                    } else {
                        push_header(&mut headers, name, val);
                    }
                }
            }
            "-d" | "--data" | "--data-binary" => {
                let v = value.unwrap_or_default();
                if v.starts_with('@') {
                    return Err(BeamError::Validation {
                        message: format!("@filename arguments are not supported in v1: {v}"),
                    });
                }
                body_text = Some(v);
                has_data = true;
            }
            "--data-raw" => {
                let v = value.unwrap_or_default();
                body_text = Some(v);
                has_data = true;
            }
            "--data-urlencode" => {
                let v = value.unwrap_or_default();
                if v.starts_with('@') {
                    return Err(BeamError::Validation {
                        message: format!("@filename arguments are not supported in v1: {v}"),
                    });
                }
                let (name, val) = match v.split_once('=') {
                    Some((n, v)) => (n.to_string(), v.to_string()),
                    None => (v.to_string(), String::new()),
                };
                urlencoded_fields.push(QueryParamField {
                    name,
                    value: val,
                    enabled: true,
                    description: None,
                });
                has_data = true;
            }
            "-F" | "--form" => {
                let v = value.unwrap_or_default();
                if let Some((n, val)) = v.split_once('=') {
                    if val.starts_with('@') {
                        // binary uploads unsupported — skip silently
                    } else {
                        form_fields.push(QueryParamField {
                            name: n.to_string(),
                            value: val.to_string(),
                            enabled: true,
                            description: None,
                        });
                    }
                }
            }
            "-u" | "--user" => {
                let v = value.unwrap_or_default();
                let (user, pass) = match v.split_once(':') {
                    Some((u, p)) => (u.to_string(), p.to_string()),
                    None => (v, String::new()),
                };
                auth = AuthConfig::Basic {
                    username: Some(user),
                    password: Some(pass),
                };
            }
            "--cookie" | "-b" => {
                let v = value.unwrap_or_default();
                cookie = Some(v);
            }
            "--url" => {
                if let Some(u) = value {
                    url = Some(u);
                }
            }
            // Ignored flags
            "-L" | "--location" | "--compressed" | "--insecure" | "-k" | "-i" | "-O" | "-o"
            | "-D" => {}
            _ => {
                // unknown flag — ignore
            }
        }
        i += 1;
    }

    if let Some(cookie_val) = cookie {
        push_header(&mut headers, "Cookie".to_string(), cookie_val);
    }

    let method = match method {
        Some(m) => m,
        None => {
            if has_data {
                HttpMethod::Post
            } else {
                HttpMethod::Get
            }
        }
    };

    let body = build_body(
        body_text.as_deref(),
        content_type.as_deref(),
        &form_fields,
        &urlencoded_fields,
    );
    let (url, query) = split_url_query(url.unwrap_or_default());

    Ok(CurlPlan {
        method,
        url,
        headers,
        query,
        body,
        auth,
    })
}

fn split_url_query(url: String) -> (String, Vec<QueryParamField>) {
    let before_fragment_end = url.find('#').unwrap_or(url.len());
    let Some(query_start) = url[..before_fragment_end].find('?') else {
        return (url, Vec::new());
    };

    let query_text = &url[query_start + 1..before_fragment_end];
    let query = if query_text.is_empty() {
        Vec::new()
    } else {
        query_text
            .split('&')
            .map(|pair| {
                let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                QueryParamField {
                    name: name.to_string(),
                    value: value.to_string(),
                    enabled: true,
                    description: None,
                }
            })
            .collect()
    };

    let mut url_without_query = String::with_capacity(url.len());
    url_without_query.push_str(&url[..query_start]);
    url_without_query.push_str(&url[before_fragment_end..]);
    (url_without_query, query)
}

fn push_header(headers: &mut Vec<HeaderField>, name: String, value: String) {
    headers.retain(|h| !h.name.eq_ignore_ascii_case(&name));
    headers.push(HeaderField {
        name,
        value,
        enabled: true,
        description: None,
    });
}

fn build_body(
    body_text: Option<&str>,
    content_type: Option<&str>,
    form_fields: &[QueryParamField],
    urlencoded_fields: &[QueryParamField],
) -> BodyConfig {
    if !form_fields.is_empty() {
        return BodyConfig::Multipart {
            fields: form_fields.to_vec(),
        };
    }
    if !urlencoded_fields.is_empty() {
        let mut fields = urlencoded_fields.to_vec();
        if let Some(text) = body_text {
            for pair in text.split('&') {
                let (name, val) = match pair.split_once('=') {
                    Some((n, v)) => (n.to_string(), v.to_string()),
                    None => (pair.to_string(), String::new()),
                };
                fields.push(QueryParamField {
                    name,
                    value: val,
                    enabled: true,
                    description: None,
                });
            }
        }
        return BodyConfig::FormUrlEncoded { fields };
    }
    match body_text {
        None => BodyConfig::None,
        Some(text) => {
            let ct = content_type.map(|c| c.to_lowercase());
            if ct.as_deref() == Some("application/json")
                || ct.as_deref().map(|c| c.contains("json")).unwrap_or(false)
            {
                BodyConfig::Json {
                    text: text.to_string(),
                }
            } else if ct.as_deref().map(|c| c.contains("xml")).unwrap_or(false) {
                BodyConfig::Xml {
                    text: text.to_string(),
                }
            } else if ct
                .as_deref()
                .map(|c| c.contains("x-www-form-urlencoded"))
                .unwrap_or(false)
            {
                let mut fields = Vec::new();
                for pair in text.split('&') {
                    let (name, val) = match pair.split_once('=') {
                        Some((n, v)) => (n.to_string(), v.to_string()),
                        None => (pair.to_string(), String::new()),
                    };
                    fields.push(QueryParamField {
                        name,
                        value: val,
                        enabled: true,
                        description: None,
                    });
                }
                BodyConfig::FormUrlEncoded { fields }
            } else {
                BodyConfig::Raw {
                    media_type: content_type.map(|s| s.to_string()),
                    text: text.to_string(),
                }
            }
        }
    }
}

fn parse_method(s: &str) -> Option<HttpMethod> {
    match s.trim().to_uppercase().as_str() {
        "GET" => Some(HttpMethod::Get),
        "POST" => Some(HttpMethod::Post),
        "PUT" => Some(HttpMethod::Put),
        "DELETE" => Some(HttpMethod::Delete),
        "PATCH" => Some(HttpMethod::Patch),
        "HEAD" => Some(HttpMethod::Head),
        "OPTIONS" => Some(HttpMethod::Options),
        "QUERY" => Some(HttpMethod::Query),
        _ => None,
    }
}

/// Returns `(flag_name, optional_inline_value)`.
/// `-XPOST` → `("-X", Some("POST"))`.
/// `--request=POST` → `("--request", Some("POST"))`.
fn split_flag(tok: &str) -> (&str, Option<&str>) {
    if let Some(long) = tok.strip_prefix("--") {
        if let Some(eq) = long.find('=') {
            return (&tok[..2 + eq], Some(&long[eq + 1..]));
        }
        return (tok, None);
    }
    if tok.starts_with('-') && tok.len() > 2 {
        return (&tok[..2], Some(&tok[2..]));
    }
    (tok, None)
}

fn takes_value(flag: &str) -> bool {
    matches!(
        flag,
        "-X" | "--request"
            | "-H"
            | "--header"
            | "-d"
            | "--data"
            | "--data-raw"
            | "--data-binary"
            | "--data-urlencode"
            | "-F"
            | "--form"
            | "-u"
            | "--user"
            | "--cookie"
            | "-b"
            | "--url"
            | "-o"
            | "-D"
    )
}

/// Tokenize a cURL command into shell-like tokens, respecting single/double
/// quotes and `\` line continuations.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut quote: Option<char> = None;

    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                current.push(c);
            }
            continue;
        }

        match c {
            '\\' => match chars.peek() {
                Some('\n') => {
                    chars.next();
                }
                Some('\r') => {
                    chars.next();
                    if matches!(chars.peek(), Some('\n')) {
                        chars.next();
                    }
                }
                // Recover Postman's `\\\n--flag` continuation after it becomes `\\--flag`.
                Some('-') if current.is_empty() => {}
                _ => current.push('\\'),
            },
            '\'' | '"' => {
                quote = Some(c);
            }
            c if c.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AuthConfig, BodyConfig, HttpMethod};

    #[test]
    fn is_curl_detects_command() {
        assert!(is_curl("curl https://example.com"));
        assert!(is_curl("CURL https://example.com"));
        assert!(is_curl("  curl https://example.com"));
        assert!(is_curl("curl \\\n  -H 'X: y'"));
        assert!(!is_curl("https://example.com"));
        assert!(!is_curl(""));
        assert!(!is_curl("   "));
    }

    #[test]
    fn parses_get_with_one_header() {
        let plan = parse(r#"curl https://example.com -H "X-Foo: bar""#).unwrap();
        assert_eq!(plan.method, HttpMethod::Get);
        assert_eq!(plan.url, "https://example.com");
        assert_eq!(plan.headers.len(), 1);
        assert_eq!(plan.headers[0].name, "X-Foo");
        assert_eq!(plan.headers[0].value, "bar");
        assert!(plan.headers[0].enabled);
        assert!(matches!(plan.body, BodyConfig::None));
        assert!(matches!(plan.auth, AuthConfig::None));
    }

    #[test]
    fn parses_post_json() {
        let cmd = r#"curl -X POST -H 'Content-Type: application/json' -d '{"foo":"bar"}' https://example.com"#;
        let plan = parse(cmd).unwrap();
        assert_eq!(plan.method, HttpMethod::Post);
        assert_eq!(plan.url, "https://example.com");
        assert_eq!(plan.headers.len(), 1);
        assert_eq!(plan.headers[0].name, "Content-Type");
        assert_eq!(plan.headers[0].value, "application/json");
        match &plan.body {
            BodyConfig::Json { text } => assert_eq!(text, "{\"foo\":\"bar\"}"),
            other => panic!("expected Json body, got {:?}", other),
        }
        assert!(matches!(plan.auth, AuthConfig::None));
    }

    #[test]
    fn parses_basic_auth() {
        let plan = parse(r#"curl -u user:pass https://example.com"#).unwrap();
        match &plan.auth {
            AuthConfig::Basic { username, password } => {
                assert_eq!(username.as_deref(), Some("user"));
                assert_eq!(password.as_deref(), Some("pass"));
            }
            other => panic!("expected Basic auth, got {:?}", other),
        }
        assert_eq!(plan.method, HttpMethod::Get);
        assert_eq!(plan.url, "https://example.com");
    }

    #[test]
    fn cookie_last_wins_with_cookie_flag_then_header() {
        let plan = parse(r#"curl --cookie "a=b" -H "Cookie: c=d" https://example.com"#).unwrap();
        let cookie_h = plan
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("Cookie"))
            .expect("cookie header present");
        assert_eq!(cookie_h.value, "c=d");
    }

    #[test]
    fn cookie_last_wins_with_header_then_cookie_flag() {
        let plan = parse(r#"curl -H "Cookie: c=d" --cookie "a=b" https://example.com"#).unwrap();
        let cookie_h = plan
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("Cookie"))
            .expect("cookie header present");
        assert_eq!(cookie_h.value, "a=b");
    }

    #[test]
    fn data_at_file_errors() {
        let result = parse(r#"curl -d @file.txt https://example.com"#);
        assert!(result.is_err());

        let result = parse(r#"curl --data-binary @file.txt https://example.com"#);
        assert!(result.is_err());
    }

    #[test]
    fn multiline_with_backslash_continuation() {
        let cmd = "curl https://example.com \\\n  -H \"X-Foo: bar\" \\\n  -X POST";
        let plan = parse(cmd).unwrap();
        assert_eq!(plan.method, HttpMethod::Post);
        assert_eq!(plan.url, "https://example.com");
        assert_eq!(plan.headers.len(), 1);
        assert_eq!(plan.headers[0].name, "X-Foo");
        assert_eq!(plan.headers[0].value, "bar");
    }

    #[test]
    fn parses_postman_multiline_command() {
        let cmd = "curl --location 'http://localhost:8081/buildings' \\
--header 'aa: bbb1' \\
--header 'Content-Type: text/plain' \\
--data '{\n\t\"address\": \"Address 2\",\n\t\"number_of_floors\": 5\n}'";
        let plan = parse(cmd).unwrap();
        assert_eq!(plan.method, HttpMethod::Post);
        assert_eq!(plan.url, "http://localhost:8081/buildings");
        assert_eq!(plan.headers.len(), 2);
        assert_eq!(plan.headers[0].name, "aa");
        assert_eq!(plan.headers[0].value, "bbb1");
        assert_eq!(plan.headers[1].name, "Content-Type");
        assert_eq!(plan.headers[1].value, "text/plain");
    }

    #[test]
    fn parses_postman_command_after_single_line_input_removes_newlines() {
        let cmd = concat!(
            "curl --location 'http://localhost:8081/buildings' \\",
            "\n",
            "--header 'aa: bbb1' \\",
            "\n",
            "--header 'Content-Type: text/plain' \\",
            "\n",
            "--data '{",
            "\n",
            "\t\"address\": \"Address 2\",",
            "\n",
            "\t\"number_of_floors\": 5",
            "\n",
            "}'",
        )
        .replace(['\n', '\r'], "");
        let plan = parse(&cmd).unwrap();

        assert_eq!(plan.method, HttpMethod::Post);
        assert_eq!(plan.url, "http://localhost:8081/buildings");
        assert_eq!(plan.headers.len(), 2);
        assert_eq!(plan.headers[0].name, "aa");
        assert_eq!(plan.headers[0].value, "bbb1");
        assert_eq!(plan.headers[1].name, "Content-Type");
        assert_eq!(plan.headers[1].value, "text/plain");
    }

    #[test]
    fn data_defaults_to_post_when_no_method() {
        let plan = parse(r#"curl -d "foo=bar" https://example.com"#).unwrap();
        assert_eq!(plan.method, HttpMethod::Post);
    }

    #[test]
    fn explicit_method_overrides_data_default() {
        let plan = parse(r#"curl -d "foo=bar" -X PUT https://example.com"#).unwrap();
        assert_eq!(plan.method, HttpMethod::Put);
    }

    #[test]
    fn duplicate_headers_last_wins() {
        let plan = parse(r#"curl -H "X-Foo: a" -H "X-Foo: b" https://example.com"#).unwrap();
        let foo = plan
            .headers
            .iter()
            .filter(|h| h.name == "X-Foo")
            .collect::<Vec<_>>();
        assert_eq!(foo.len(), 1);
        assert_eq!(foo[0].value, "b");
    }

    #[test]
    fn url_flag_sets_url() {
        let plan = parse("curl --url https://example.com").unwrap();
        assert_eq!(plan.url, "https://example.com");
    }

    #[test]
    fn url_query_is_moved_into_params() {
        let plan = parse(r#"curl --url 'https://httpbingo.org/get?aaa=111&='"#).unwrap();

        assert_eq!(plan.url, "https://httpbingo.org/get");
        assert_eq!(
            plan.query,
            vec![
                QueryParamField {
                    name: "aaa".to_string(),
                    value: "111".to_string(),
                    enabled: true,
                    description: None,
                },
                QueryParamField {
                    name: String::new(),
                    value: String::new(),
                    enabled: true,
                    description: None,
                },
            ]
        );
    }

    #[test]
    fn url_query_preserves_duplicates_encoding_and_fragment() {
        let plan =
            parse(r#"curl 'https://example.com/items?tag=one&tag=two%20words&flag#results'"#)
                .unwrap();

        assert_eq!(plan.url, "https://example.com/items#results");
        assert_eq!(
            plan.query
                .iter()
                .map(|param| (param.name.as_str(), param.value.as_str()))
                .collect::<Vec<_>>(),
            vec![("tag", "one"), ("tag", "two%20words"), ("flag", "")]
        );
    }

    #[test]
    fn question_mark_in_fragment_is_not_treated_as_query() {
        let plan = parse(r#"curl 'https://example.com/items#results?view=full'"#).unwrap();

        assert_eq!(plan.url, "https://example.com/items#results?view=full");
        assert!(plan.query.is_empty());
    }

    #[test]
    fn form_flag_builds_multipart() {
        let plan = parse(r#"curl -F "name=value" -F "file=@/tmp/x" https://example.com"#).unwrap();
        match &plan.body {
            BodyConfig::Multipart { fields } => {
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].name, "name");
                assert_eq!(fields[0].value, "value");
            }
            other => panic!("expected Multipart, got {:?}", other),
        }
    }

    #[test]
    fn ignored_flags_do_not_error() {
        let cmd = "curl -L -k --compressed --insecure -i https://example.com";
        let plan = parse(cmd).unwrap();
        assert_eq!(plan.url, "https://example.com");
        assert_eq!(plan.method, HttpMethod::Get);
    }

    #[test]
    fn unknown_flag_is_ignored() {
        let plan = parse(r#"curl --future-flag https://example.com"#).unwrap();
        assert_eq!(plan.url, "https://example.com");
    }

    #[test]
    fn data_urlencode_builds_form_body() {
        let plan = parse(r#"curl --data-urlencode "name=value" https://example.com"#).unwrap();
        match &plan.body {
            BodyConfig::FormUrlEncoded { fields } => {
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].name, "name");
                assert_eq!(fields[0].value, "value");
            }
            other => panic!("expected FormUrlEncoded, got {:?}", other),
        }
        assert_eq!(plan.method, HttpMethod::Post);
    }

    #[test]
    fn form_urlencoded_via_content_type() {
        let cmd = r#"curl -H "Content-Type: application/x-www-form-urlencoded" -d "a=1&b=2" https://example.com"#;
        let plan = parse(cmd).unwrap();
        match &plan.body {
            BodyConfig::FormUrlEncoded { fields } => assert_eq!(fields.len(), 2),
            other => panic!("expected FormUrlEncoded, got {:?}", other),
        }
    }

    #[test]
    fn data_raw_with_at_is_literal_not_error() {
        // --data-raw does not interpret @file, so it should pass through as text.
        let plan = parse(r#"curl --data-raw "@file.txt" https://example.com"#).unwrap();
        match &plan.body {
            BodyConfig::Raw { text, .. } => assert_eq!(text, "@file.txt"),
            other => panic!("expected Raw, got {:?}", other),
        }
    }

    #[test]
    fn xml_body_via_content_type() {
        let cmd = r#"curl -H "Content-Type: application/xml" -d "<a>1</a>" https://example.com"#;
        let plan = parse(cmd).unwrap();
        match &plan.body {
            BodyConfig::Xml { text } => assert_eq!(text, "<a>1</a>"),
            other => panic!("expected Xml, got {:?}", other),
        }
    }

    #[test]
    fn short_flag_with_inline_value() {
        let plan = parse(
            r#"curl -XPOST -H'Content-Type: application/json' -d'{"a":1}' https://example.com"#,
        )
        .unwrap();
        assert_eq!(plan.method, HttpMethod::Post);
        assert_eq!(plan.headers[0].name, "Content-Type");
        match &plan.body {
            BodyConfig::Json { text } => assert_eq!(text, "{\"a\":1}"),
            other => panic!("expected Json, got {:?}", other),
        }
    }

    #[test]
    fn long_flag_with_equals_value() {
        let plan = parse(r#"curl --request=PATCH https://example.com"#).unwrap();
        assert_eq!(plan.method, HttpMethod::Patch);
    }

    #[test]
    fn user_without_colon_treats_all_as_username() {
        let plan = parse(r#"curl -u justuser https://example.com"#).unwrap();
        match &plan.auth {
            AuthConfig::Basic { username, password } => {
                assert_eq!(username.as_deref(), Some("justuser"));
                assert_eq!(password.as_deref(), Some(""));
            }
            other => panic!("expected Basic, got {:?}", other),
        }
    }

    #[test]
    fn empty_command_errors() {
        assert!(parse("").is_err());
    }

    #[test]
    fn non_curl_first_token_errors() {
        assert!(parse("wget https://example.com").is_err());
    }
}
