//! HTTP client construction.

use std::time::Duration;

/// Builds a client emulating Chrome 153 on Windows.
///
/// The emulation profile supplies the TLS/JA3/JA4 and HTTP/2 signature; the
/// profile's built-in headers are disabled so [`crate::headers`] owns the exact
/// header list and ordering. When `proxy` is given it is applied to all traffic.
pub fn build_client(proxy: Option<&str>) -> wreq::Result<wreq::Client> {
    let emulation = wreq_util::Emulation::builder()
        .profile(wreq_util::Profile::Chrome153)
        .platform(wreq_util::Platform::Windows)
        .headers(false)
        .build();

    let mut builder = wreq::Client::builder()
        .emulation(emulation)
        .timeout(Duration::from_secs(30));

    if let Some(proxy) = proxy {
        builder = builder.proxy(wreq::Proxy::all(proxy)?);
    }

    builder.build()
}
