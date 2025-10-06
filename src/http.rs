use crate::types::{HttpMethod, RequestConfig, ResponseData};
use std::time::Instant;

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
    
    let client = reqwest::Client::new();
    
    // Build the request
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
        let body_text = config.body.text();
        if !body_text.is_empty() {
            request_builder = request_builder.body(body_text);
            
            // Set content type if not already set
            if !config.headers.iter().any(|(k, _)| k.to_lowercase() == "content-type") {
                request_builder = request_builder.header("Content-Type", &config.content_type);
            }
        }
    }

    // Send the request
    match request_builder.send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            let status_text = response.status().canonical_reason().unwrap_or("Unknown").to_string();
            
            // Extract headers
            let mut headers = Vec::new();
            for (name, value) in response.headers() {
                if let Ok(value_str) = value.to_str() {
                    headers.push((name.to_string(), value_str.to_string()));
                }
            }

            // Get response body
            let body = match response.text().await {
                Ok(text) => text,
                Err(e) => return Err(format!("Failed to read response body: {}", e)),
            };

            let elapsed = start_time.elapsed();
            let size = body.len();

            Ok(ResponseData {
                status,
                status_text,
                headers,
                body,
                size,
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
    
    // Add body for POST, PUT, PATCH requests
    if matches!(config.method, HttpMethod::POST | HttpMethod::PUT | HttpMethod::PATCH) {
        let body_text = config.body.text();
        if !body_text.is_empty() {
            curl_parts.push("-d".to_string());
            curl_parts.push(format!("'{}'", body_text.replace("'", "'\\''")));
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