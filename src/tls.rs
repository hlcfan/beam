//! TLS initialization shared by HTTP requests and update downloads.
//!
//! `run_app` calls `init_provider` once at startup, before any client is built.

#[cfg(not(target_os = "macos"))]
use std::sync::Once;

pub(crate) fn init_provider() {
    #[cfg(not(target_os = "macos"))]
    {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            // Keep a provider that another client has already installed.
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_http_clients_initialize() {
        init_provider();
        init_provider();
        #[cfg(not(target_os = "macos"))]
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
        reqwest::Client::builder()
            .build()
            .expect("async HTTPS client");
        reqwest::blocking::Client::builder()
            .build()
            .expect("blocking HTTPS client");
    }

    #[test]
    #[ignore = "requires internet access"]
    fn both_http_clients_validate_https() {
        init_provider();
        let timeout = std::time::Duration::from_secs(15);
        reqwest::blocking::Client::builder()
            .timeout(timeout)
            .build()
            .expect("blocking HTTPS client")
            .get("https://example.com/")
            .send()
            .expect("blocking TLS handshake")
            .error_for_status()
            .expect("blocking HTTPS response");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        runtime.block_on(async {
            reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .expect("async HTTPS client")
                .get("https://example.com/")
                .send()
                .await
                .expect("async TLS handshake")
                .error_for_status()
                .expect("async HTTPS response");
        });
    }
}
