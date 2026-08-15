use std::sync::Once;

pub fn http_client() -> reqwest::Client {
    static INSTALL_PROVIDER: Once = Once::new();
    INSTALL_PROVIDER.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("the Rustls crypto provider should only be installed once");
    });
    reqwest::Client::new()
}
