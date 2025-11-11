use crate::types::{HttpMethod, RequestConfig, ResponseData, AuthType};
use std::time::Instant;
use base64::{Engine as _, engine::general_purpose};
use log::{info};

fn is_binary_content_type(content_type: &str) -> bool {
    let content_type_lower = content_type.to_lowercase();

    // Check for text-based content types
    if content_type_lower.starts_with("text/") ||
       content_type_lower.starts_with("application/json") ||
       content_type_lower.starts_with("application/xml") ||
       content_type_lower.starts_with("application/javascript") ||
       content_type_lower.starts_with("application/x-www-form-urlencoded") ||
       content_type_lower.contains("charset") {
        return false;
    }

    // Check for known binary content types
    if content_type_lower.starts_with("image/") ||
       content_type_lower.starts_with("video/") ||
       content_type_lower.starts_with("audio/") ||
       content_type_lower.starts_with("application/octet-stream") ||
       content_type_lower.starts_with("application/pdf") ||
       content_type_lower.starts_with("application/zip") ||
       content_type_lower.starts_with("application/x-") {
        return true;
    }

    // Default to binary for unknown types
    true
}

pub async fn send_request(config: RequestConfig) -> Result<ResponseData, String> {
    let start_time = Instant::now();

    // Validate URL
    if config.url.trim().is_empty() {
        return Err("URL cannot be empty".to_string());
    }

    // Basic URL validation
    if !config.url.starts_with("http://") && !config.url.starts_with("https://") {
        return Err("URL must start with http:// or https://".to_string());
    }

    // TODO: reuse the client
    let client = reqwest::Client::new();

    // Build the request
    // TODO: use client.request directly instead of match clause
    let mut request_builder = match config.method {
        HttpMethod::GET => client.get(&config.url),
        HttpMethod::POST => client.post(&config.url),
        HttpMethod::PUT => client.put(&config.url),
        HttpMethod::DELETE => client.delete(&config.url),
        HttpMethod::PATCH => client.patch(&config.url),
        HttpMethod::HEAD => client.head(&config.url),
        HttpMethod::OPTIONS => {
            return Err("OPTIONS method not supported yet".to_string());
        }
    };

    // Add headers
    for (key, value) in &config.headers {
        if !key.is_empty() && !value.is_empty() {
            request_builder = request_builder.header(key, value);
        }
    }

    // Add authentication
    match config.auth_type {
        AuthType::None => {
            // No authentication needed
        }
        AuthType::Bearer => {
            if !config.bearer_token.is_empty() {
                let auth_header = format!("Bearer {}", config.bearer_token);
                request_builder = request_builder.header("Authorization", auth_header);
            }
        }
        AuthType::Basic => {
            if !config.basic_username.is_empty() {
                let credentials = if config.basic_password.is_empty() {
                    config.basic_username.clone()
                } else {
                    format!("{}:{}", config.basic_username, config.basic_password)
                };
                let encoded = general_purpose::STANDARD.encode(credentials);
                let auth_header = format!("Basic {}", encoded);
                request_builder = request_builder.header("Authorization", auth_header);
            }
        }
        AuthType::ApiKey => {
            if !config.api_key.is_empty() && !config.api_key_header.is_empty() {
                request_builder = request_builder.header(&config.api_key_header, &config.api_key);
            }
        }
    }

    // Add query parameters
    let mut query_params = Vec::new();
    for (key, value) in &config.params {
        if !key.is_empty() && !value.is_empty() {
            query_params.push((key, value));
        }
    }
    if !query_params.is_empty() {
        request_builder = request_builder.query(&query_params);
    }

    // Add body for POST, PUT, PATCH requests
    if matches!(config.method, HttpMethod::POST | HttpMethod::PUT | HttpMethod::PATCH) {
        if !config.body.is_empty() {
            request_builder = request_builder.body(config.body.clone());

            // Set content type if not already set
            if !config.headers.iter().any(|(k, _)| k.to_lowercase() == "content-type") {
                request_builder = request_builder.header("Content-Type", &config.content_type);
            }
        }
    }

    info!("DEBUG: sending request - {:?}", Instant::now());
    // Send the request
    match request_builder.send().await {
        Ok(response) => {
            info!("DEBUG: done sending request - {:?}", Instant::now());
            let status = response.status().as_u16();
            let status_text = response.status().canonical_reason().unwrap_or("Unknown").to_string();

            // Extract headers
            let mut headers = Vec::new();
            for (name, value) in response.headers() {
                if let Ok(value_str) = value.to_str() {
                    headers.push((name.to_string(), value_str.to_string()));
                }
            }

            // Get content type
            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|ct| ct.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_string();

            // Determine if response is binary
            let is_binary = is_binary_content_type(&content_type);

            // Get response body
            let (body, actual_size) = if is_binary {
                // For binary content, get the raw bytes and create a summary
                match response.bytes().await {
                    Ok(bytes) => {
                        let size = bytes.len();
                        let summary = format!(
                            "[Binary data: {} bytes]\nContent-Type: {}\nFirst 100 bytes (hex): {}",
                            size,
                            content_type,
                            bytes.iter()
                                .take(100)
                                .map(|b| format!("{:02x}", b))
                                .collect::<Vec<_>>()
                                .join(" ")
                        );
                        (summary, size)
                    }
                    Err(e) => return Err(format!("Failed to read binary response: {}", e)),
                }
            } else {
                // For text content, try to convert to string
                match response.text().await {
                    Ok(text) => {
                        let size = text.len();
                        (text, size)
                    }
                    Err(e) => return Err(format!("Failed to read text response: {}", e)),
                }
            };

            let elapsed = start_time.elapsed();

            Ok(ResponseData {
                status,
                status_text,
                headers,
                body,
                content_type,
                is_binary,
                size: actual_size,
                time: elapsed.as_millis() as u64,
            })
        }
        Err(e) => Err(format!("Request failed: {}", e)),
    }
}

pub fn generate_curl_command(config: &RequestConfig) -> String {
    let mut curl_parts = vec!["curl".to_string()];

    // Add method
    if config.method != HttpMethod::GET {
        curl_parts.push("-X".to_string());
        curl_parts.push(config.method.to_string());
    }

    // Add headers
    for (key, value) in &config.headers {
        if !key.is_empty() && !value.is_empty() {
            curl_parts.push("-H".to_string());
            curl_parts.push(format!("'{}: {}'", key, value));
        }
    }

    // Add authentication
    match config.auth_type {
        AuthType::None => {
            // No authentication needed
        }
        AuthType::Bearer => {
            if !config.bearer_token.is_empty() {
                curl_parts.push("-H".to_string());
                curl_parts.push(format!("'Authorization: Bearer {}'", config.bearer_token));
            }
        }
        AuthType::Basic => {
            if !config.basic_username.is_empty() {
                if config.basic_password.is_empty() {
                    curl_parts.push("-u".to_string());
                    curl_parts.push(format!("'{}'", config.basic_username));
                } else {
                    curl_parts.push("-u".to_string());
                    curl_parts.push(format!("'{}:{}'", config.basic_username, config.basic_password));
                }
            }
        }
        AuthType::ApiKey => {
            if !config.api_key.is_empty() && !config.api_key_header.is_empty() {
                curl_parts.push("-H".to_string());
                curl_parts.push(format!("'{}: {}'", config.api_key_header, config.api_key));
            }
        }
    }

    // Add body for POST, PUT, PATCH requests
    if matches!(config.method, HttpMethod::POST | HttpMethod::PUT | HttpMethod::PATCH) {
        if !config.body.is_empty() {
            curl_parts.push("-d".to_string());
            curl_parts.push(format!("'{}'", config.body.replace("'", "'\\''")));
        }
    }

    // Build URL with query parameters
    let mut url = config.url.clone();
    let query_params: Vec<String> = config.params
        .iter()
        .filter(|(k, v)| !k.is_empty() && !v.is_empty())
        .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
        .collect();

    if !query_params.is_empty() {
        if url.contains('?') {
            url.push('&');
        } else {
            url.push('?');
        }
        url.push_str(&query_params.join("&"));
    }

    curl_parts.push(format!("'{}'", url));

    curl_parts.join(" ")
}
