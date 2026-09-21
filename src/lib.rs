#![warn(missing_docs)]
//! Request-based fubo.tv account generator.
//!
//! Creates fubo.tv accounts using only HTTP requests, emulating Chrome 153 via
//! [`wreq_util`] and replaying the recorded signup flow with byte-for-byte
//! header ordering.

pub mod account;
pub mod client;
pub mod flow;
pub mod headers;
pub mod proxy;

pub use account::Account;
pub use flow::CreatedAccount;
pub use proxy::ProxyList;
