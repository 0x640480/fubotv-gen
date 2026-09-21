//! Signup flow replay and per-account cookie jar.

use anyhow::{Context, Result};
use rand::Rng;
use rand::distr::Alphanumeric;
use serde_json::{Value, json};
use wreq::header::HeaderMap;

use crate::account::Account;
use crate::headers::{self, BodyKind, SessionCtx, X_DEVICE_ID_COMPUTED};

const URL_WELCOME: &str = "https://www.fubo.tv/welcome";
const URL_SIGNUP: &str = "https://www.fubo.tv/signup";
const URL_HOME: &str = "https://www.fubo.tv/";
const URL_GTM_SET_COOKIE: &str = "https://fubotv-gtm.fubo.tv/_/set_cookie";
const URL_TATARI: &str = "https://segment.prod.bidr.io/associate-segment";
const URL_AXON_PIXEL: &str = "https://s.axon.ai/pixel.js";
const URL_LOCATION: &str = "https://api.fubo.tv/v3/location";
const URL_CONFIG: &str = "https://api.fubo.tv/papi/v1/config";
const URL_FREE_TRIAL: &str =
    "https://api.fubo.tv/gg/products/has-free-trial?seriesId=0&networkId=0";
const URL_POPULAR: &str =
    "https://api.fubo.tv/popular/unauth/v1?contentType=series&limit=10&genreIds=11";
const URL_NORMALIZE: &str = "https://api.fubo.tv/user/email/normalize";
const URL_AUTH: &str = "https://api.fubo.tv/auth";
const URL_USER: &str = "https://api.fubo.tv/user";
const URL_SETTINGS: &str = "https://api.fubo.tv/papi/v1/settings";
const URL_NAVIGATION: &str = "https://api.fubo.tv/papi/v1/main-navigation";

/// A successfully created account plus its credentials.
#[derive(Debug, serde::Serialize)]
pub struct CreatedAccount {
    /// Account email.
    pub email: String,
    /// Account password.
    pub password: String,
    /// fubo user id.
    pub user_id: String,
    /// Device id used for the account.
    pub device_id: String,
    /// Bearer access token.
    pub access_token: String,
    /// Refresh token.
    pub refresh_token: String,
    /// Access token lifetime in seconds.
    pub expires_in: i64,
    /// Creation timestamp (RFC 3339).
    pub created_at: String,
}

#[derive(Debug)]
struct Cookie {
    name: String,
    value: String,
    domain: String,
    path: String,
}

/// Minimal domain/path-scoped cookie store.
#[derive(Debug, Default)]
pub struct CookieJar {
    entries: Vec<Cookie>,
}

impl CookieJar {
    /// Stores every `set-cookie` from `resp`, scoped to the request host.
    fn absorb(&mut self, resp: &wreq::Response, url: &url::Url) {
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        let values: Vec<String> = resp
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .map(str::to_owned)
            .collect();

        for raw in values {
            let mut parts = raw.split(';');
            let Some(pair) = parts.next() else { continue };
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let mut domain = host.clone();
            let mut path = String::from("/");
            for attr in parts {
                let attr = attr.trim();
                if let Some(d) = attr
                    .strip_prefix("domain=")
                    .or_else(|| attr.strip_prefix("Domain="))
                {
                    domain = d.trim_start_matches('.').to_ascii_lowercase();
                } else if let Some(p) = attr
                    .strip_prefix("path=")
                    .or_else(|| attr.strip_prefix("Path="))
                {
                    path = p.trim().to_owned();
                }
            }
            self.entries
                .retain(|c| !(c.name == name && c.domain == domain));
            self.entries.push(Cookie {
                name: name.to_owned(),
                value: value.trim().to_owned(),
                domain,
                path,
            });
        }
    }

    /// Serialized `Cookie` header for `url`, or `None` when nothing matches.
    fn header_for(&self, url: &url::Url) -> Option<String> {
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        let path = url.path();
        let joined = self
            .entries
            .iter()
            .filter(|c| {
                (host == c.domain || host.ends_with(&format!(".{}", c.domain)))
                    && path.starts_with(c.path.as_str())
            })
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ");
        (!joined.is_empty()).then_some(joined)
    }

    /// Value of the first cookie with `name`.
    fn get(&self, name: &str) -> Option<String> {
        self.entries
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.value.clone())
    }
}

/// Mutable state for one account's flow.
pub struct SessionState {
    device: String,
    session_id: String,
    ad_id: String,
    postal: String,
    country: String,
    access_token: Option<String>,
    user_id: Option<String>,
    jar: CookieJar,
}

impl SessionState {
    /// Creates state with fresh per-account identifiers.
    fn new() -> Self {
        let mut rng = rand::rng();
        Self {
            device: String::new(),
            session_id: format!("{}-{}", alnum(&mut rng, 8), alnum(&mut rng, 9)),
            ad_id: alnum(&mut rng, 18),
            postal: String::from("32136"),
            country: String::from("USA"),
            access_token: None,
            user_id: None,
            jar: CookieJar::default(),
        }
    }
}

fn alnum(rng: &mut impl Rng, n: usize) -> String {
    (0..n).map(|_| rng.sample(Alphanumeric) as char).collect()
}

async fn send(
    client: &wreq::Client,
    state: &mut SessionState,
    method: wreq::Method,
    url: &str,
    build: fn(&SessionCtx<'_>) -> Result<HeaderMap>,
    body: Option<Value>,
    referer: &str,
) -> Result<wreq::Response> {
    let body_kind = if body.is_some() {
        BodyKind::Json
    } else {
        BodyKind::None
    };
    let parsed = url::Url::parse(url)?;

    for attempt in 1..=3u8 {
        let cookie = state.jar.header_for(&parsed);
        let ctx = SessionCtx {
            device: &state.device,
            session_id: &state.session_id,
            ad_id: &state.ad_id,
            referer,
            postal: &state.postal,
            country: &state.country,
            access_token: state.access_token.as_deref(),
            user_id: state.user_id.as_deref(),
            cookie_hdr: cookie.as_deref(),
            body_kind,
        };

        let mut rb = client.request(method.clone(), url).headers(build(&ctx)?);
        if let Some(v) = body.as_ref() {
            rb = rb.body(serde_json::to_vec(v)?);
        }

        let resp = rb.send().await?;
        state.jar.absorb(&resp, &parsed);

        if resp.status() == wreq::StatusCode::TOO_MANY_REQUESTS && attempt < 3 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            continue;
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            let snippet: String = text.chars().take(300).collect();
            anyhow::bail!("{method} {url} -> {status}: {snippet}");
        }
        return Ok(resp);
    }
    unreachable!("loop returns or bails")
}

/// Runs the full signup flow, returning the created account credentials.
///
/// `account` may have its email regenerated if the address is already taken.
pub async fn create_account(
    client: &wreq::Client,
    account: &mut Account,
) -> Result<CreatedAccount> {
    let mut state = SessionState::new();

    // 1. Landing page mints the device id and visitor cookies.
    let resp = send(
        client,
        &mut state,
        wreq::Method::GET,
        URL_WELCOME,
        headers::doc_welcome,
        None,
        headers::REFERER,
    )
    .await?;
    state.device = resp
        .headers()
        .get(X_DEVICE_ID_COMPUTED)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .or_else(|| state.jar.get("ftvOption%3AuniqueId"))
        .context("device id not supplied by /welcome")?;

    // 2. Signup page.
    send(
        client,
        &mut state,
        wreq::Method::GET,
        URL_SIGNUP,
        headers::doc_signup,
        None,
        "https://www.fubo.tv/welcome",
    )
    .await?;

    // 3. Best-effort analytics/anti-bot cookie bootstrap.
    let mut rng = rand::rng();
    let _ = send(
        client,
        &mut state,
        wreq::Method::GET,
        &format!("{URL_GTM_SET_COOKIE}?val={}&path=/", alnum(&mut rng, 40)),
        headers::api_short,
        None,
        headers::REFERER,
    )
    .await;
    let _ = send(
        client,
        &mut state,
        wreq::Method::GET,
        &format!(
            "{URL_TATARI}?buzz_key=tatari&segment_key=tatari-938&value=&uncacheplz={}",
            rng.random_range(1_000_000_000u32..u32::MAX)
        ),
        headers::api_short,
        None,
        headers::REFERER,
    )
    .await;
    let _ = send(
        client,
        &mut state,
        wreq::Method::GET,
        URL_AXON_PIXEL,
        headers::api_short,
        None,
        headers::REFERER,
    )
    .await;
    let _ = send(
        client,
        &mut state,
        wreq::Method::GET,
        URL_HOME,
        headers::doc_welcome,
        None,
        headers::REFERER,
    )
    .await;

    // 4. Geo/entry bootstrap.
    let location: Value = send(
        client,
        &mut state,
        wreq::Method::GET,
        URL_LOCATION,
        headers::api_full,
        None,
        headers::REFERER,
    )
    .await?
    .json()
    .await?;
    if let Some(p) = location["postal"].as_str() {
        state.postal = p.to_owned();
    }
    if let Some(c) = location["country_code"].as_str() {
        state.country = c.to_owned();
    }

    send(
        client,
        &mut state,
        wreq::Method::GET,
        URL_CONFIG,
        headers::api_full,
        None,
        headers::REFERER,
    )
    .await?;

    let _ = send(
        client,
        &mut state,
        wreq::Method::GET,
        URL_FREE_TRIAL,
        headers::api_short,
        None,
        headers::REFERER,
    )
    .await;
    let _ = send(
        client,
        &mut state,
        wreq::Method::GET,
        URL_POPULAR,
        headers::api_short,
        None,
        headers::REFERER,
    )
    .await;

    // 5/6. Normalize then create, regenerating the address while taken/invalid.
    let mut auth = Value::Null;
    for attempt in 0..8 {
        let resp = send(
            client,
            &mut state,
            wreq::Method::POST,
            URL_NORMALIZE,
            headers::api_full,
            Some(json!({ "email": account.email })),
            headers::REFERER,
        )
        .await?;
        let normalized: Value = resp.json().await?;
        if normalized["exists"].as_bool() == Some(true) {
            account.regenerate_email();
            continue;
        }

        let auth_body = json!({
            "email": account.email,
            "password": account.password,
            "homePostalCode": state.postal,
            "terms": false,
            "anonymous_id": state.device,
        });
        match send(
            client,
            &mut state,
            wreq::Method::POST,
            URL_AUTH,
            headers::api_full,
            Some(auth_body),
            headers::REFERER,
        )
        .await
        {
            Ok(resp) => {
                auth = resp.json().await?;
                break;
            }
            Err(error) if error.to_string().contains("email address is not valid") => {
                if attempt == 7 {
                    return Err(error);
                }
                account.regenerate_email();
            }
            Err(error) => return Err(error),
        }
    }
    anyhow::ensure!(auth != Value::Null, "/auth produced no response");

    let data = &auth["data"];
    let tokens = &data["authTokens"];
    let created_email = data["email"]
        .as_str()
        .unwrap_or(account.email.as_str())
        .to_owned();
    account.email = created_email.clone();
    let access_token = tokens["access_token"]
        .as_str()
        .context("/auth returned no access_token")?
        .to_owned();
    let refresh_token = tokens["refresh_token"]
        .as_str()
        .context("/auth returned no refresh_token")?
        .to_owned();
    let expires_in = tokens["expires_in"].as_i64().unwrap_or_default();
    state.user_id = data["id"].as_str().map(str::to_owned);
    state.access_token = Some(access_token.clone());

    // 7. Confirm the session against an authenticated endpoint.
    let user: Value = send(
        client,
        &mut state,
        wreq::Method::GET,
        URL_USER,
        headers::api_full,
        None,
        headers::REFERER,
    )
    .await?
    .json()
    .await?;
    anyhow::ensure!(
        user["data"]["email"].as_str() == Some(created_email.as_str()),
        "/user did not return the created email"
    );

    // 8. Optional post-auth warmup.
    let _ = send(
        client,
        &mut state,
        wreq::Method::GET,
        URL_SETTINGS,
        headers::api_full,
        None,
        headers::REFERER,
    )
    .await;
    let _ = send(
        client,
        &mut state,
        wreq::Method::GET,
        URL_NAVIGATION,
        headers::api_full,
        None,
        headers::REFERER,
    )
    .await;

    Ok(CreatedAccount {
        email: account.email.clone(),
        password: account.password.clone(),
        user_id: state.user_id.clone().unwrap_or_default(),
        device_id: state.device.clone(),
        access_token,
        refresh_token,
        expires_in,
        created_at: chrono::Utc::now().to_rfc3339(),
    })
}
