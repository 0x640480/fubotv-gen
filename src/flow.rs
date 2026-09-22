//! Signup flow replay, expressed as an explicit state machine, plus a
//! per-account cookie jar.
//!
//! [`create_account`] drives a [`Step`] cursor: every step is a single, named
//! network action that returns the step to run next. Request order, retry
//! behaviour and best-effort hits match the recorded wire trace exactly.

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

/// Normalize+create attempts before the address is considered uncreatable.
const MAX_AUTH_ATTEMPTS: u8 = 8;

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

/// A fire-and-forget request whose failure never aborts the flow.
///
/// Each variant knows its own URL, headers and cache-buster.
#[derive(Clone, Copy, Debug)]
enum Noise {
    GtmSetCookie,
    Tatari,
    AxonPixel,
    Home,
    FreeTrial,
    Popular,
    Settings,
    Navigation,
}

impl Noise {
    fn method(self) -> wreq::Method {
        wreq::Method::GET
    }

    fn headers(self) -> fn(&SessionCtx<'_>) -> Result<HeaderMap> {
        match self {
            Noise::Home => headers::doc_welcome,
            _ => headers::api_short,
        }
    }

    /// Target URL, minting a cache-buster where the wire trace had one.
    fn url(self) -> String {
        match self {
            Noise::GtmSetCookie => format!(
                "{URL_GTM_SET_COOKIE}?val={}&path=/",
                alnum(&mut rand::rng(), 40)
            ),
            Noise::Tatari => format!(
                "{URL_TATARI}?buzz_key=tatari&segment_key=tatari-938&value=&uncacheplz={}",
                rand::rng().random_range(1_000_000_000u32..u32::MAX)
            ),
            Noise::AxonPixel => URL_AXON_PIXEL.to_owned(),
            Noise::Home => URL_HOME.to_owned(),
            Noise::FreeTrial => URL_FREE_TRIAL.to_owned(),
            Noise::Popular => URL_POPULAR.to_owned(),
            Noise::Settings => URL_SETTINGS.to_owned(),
            Noise::Navigation => URL_NAVIGATION.to_owned(),
        }
    }
}

/// Pre-auth cookie and analytics bootstrap, right after the signup document.
const ANALYTICS: &[Noise] = &[
    Noise::GtmSetCookie,
    Noise::Tatari,
    Noise::AxonPixel,
    Noise::Home,
];

/// Pre-auth catalog probes that only warm server-side caches.
const DISCOVERY: &[Noise] = &[Noise::FreeTrial, Noise::Popular];

/// Post-auth warmup hits.
const WARMUP: &[Noise] = &[Noise::Settings, Noise::Navigation];

/// One network action in the signup flow.
///
/// A step never advances itself; it returns its successor so the cursor stays
/// in one place and the whole flow is readable top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Welcome,
    Signup,
    Analytics { index: usize },
    Location,
    Config,
    Discovery { index: usize },
    Authenticate { attempt: u8 },
    ConfirmUser,
    Warmup { index: usize },
    Done,
}

/// Credentials returned by `/auth`.
#[derive(Debug)]
struct Credentials {
    email: String,
    access_token: String,
    refresh_token: String,
    expires_in: i64,
}

/// Carries a single account through the flow.
struct Flow<'a> {
    client: &'a wreq::Client,
    account: &'a mut Account,
    session: SessionState,
    auth: Value,
    credentials: Option<Credentials>,
}

impl<'a> Flow<'a> {
    fn new(client: &'a wreq::Client, account: &'a mut Account) -> Self {
        Self {
            client,
            account,
            session: SessionState::new(),
            auth: Value::Null,
            credentials: None,
        }
    }

    /// Runs `step` and returns the next one.
    async fn advance(&mut self, step: Step) -> Result<Step> {
        match step {
            Step::Welcome => self.welcome().await,
            Step::Signup => self.signup().await,
            Step::Analytics { index } => self.analytics(index).await,
            Step::Location => self.location().await,
            Step::Config => self.config().await,
            Step::Discovery { index } => self.discovery(index).await,
            Step::Authenticate { attempt } => self.authenticate(attempt).await,
            Step::ConfirmUser => self.confirm_user().await,
            Step::Warmup { index } => self.warmup(index).await,
            Step::Done => Ok(Step::Done),
        }
    }

    // -- transport ---------------------------------------------------------

    /// Sends a request, retrying twice on `429` and failing on any non-2xx.
    async fn request(
        &mut self,
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
            let cookie = self.session.jar.header_for(&parsed);
            let ctx = SessionCtx {
                device: &self.session.device,
                session_id: &self.session.session_id,
                ad_id: &self.session.ad_id,
                referer,
                postal: &self.session.postal,
                country: &self.session.country,
                access_token: self.session.access_token.as_deref(),
                user_id: self.session.user_id.as_deref(),
                cookie_hdr: cookie.as_deref(),
                body_kind,
            };

            let mut rb = self
                .client
                .request(method.clone(), url)
                .headers(build(&ctx)?);
            if let Some(v) = body.as_ref() {
                rb = rb.body(serde_json::to_vec(v)?);
            }

            let resp = rb.send().await?;
            self.session.jar.absorb(&resp, &parsed);

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

    async fn get(
        &mut self,
        url: &str,
        headers: fn(&SessionCtx<'_>) -> Result<HeaderMap>,
    ) -> Result<wreq::Response> {
        self.request(wreq::Method::GET, url, headers, None, headers::REFERER)
            .await
    }

    async fn post_json(&mut self, url: &str, body: Value) -> Result<wreq::Response> {
        self.request(
            wreq::Method::POST,
            url,
            headers::api_full,
            Some(body),
            headers::REFERER,
        )
        .await
    }

    /// Sends a best-effort request, swallowing every error.
    async fn best_effort(&mut self, noise: Noise) {
        let _ = self
            .request(
                noise.method(),
                &noise.url(),
                noise.headers(),
                None,
                headers::REFERER,
            )
            .await;
    }

    // -- steps -------------------------------------------------------------

    /// Step 1: the landing document mints the device id.
    async fn welcome(&mut self) -> Result<Step> {
        let resp = self.get(URL_WELCOME, headers::doc_welcome).await?;
        self.session.device = resp
            .headers()
            .get(X_DEVICE_ID_COMPUTED)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
            .or_else(|| self.session.jar.get("ftvOption%3AuniqueId"))
            .context("device id not supplied by /welcome")?;
        Ok(Step::Signup)
    }

    /// Step 2: the signup document.
    async fn signup(&mut self) -> Result<Step> {
        self.request(
            wreq::Method::GET,
            URL_SIGNUP,
            headers::doc_signup,
            None,
            URL_WELCOME,
        )
        .await?;
        Ok(Step::Analytics { index: 0 })
    }

    /// Step 3: anti-bot cookie and analytics bootstrap.
    async fn analytics(&mut self, index: usize) -> Result<Step> {
        self.best_effort(ANALYTICS[index]).await;
        Ok(next(
            ANALYTICS,
            index,
            |i| Step::Analytics { index: i },
            Step::Location,
        ))
    }

    /// Step 4a: geo lookup seeds the postal code and country.
    async fn location(&mut self) -> Result<Step> {
        let location: Value = self
            .get(URL_LOCATION, headers::api_full)
            .await?
            .json()
            .await?;
        if let Some(postal) = location["postal"].as_str() {
            self.session.postal = postal.to_owned();
        }
        if let Some(country) = location["country_code"].as_str() {
            self.session.country = country.to_owned();
        }
        Ok(Step::Config)
    }

    /// Step 4b: feature config document.
    async fn config(&mut self) -> Result<Step> {
        self.get(URL_CONFIG, headers::api_full).await?;
        Ok(Step::Discovery { index: 0 })
    }

    /// Step 4c: catalog probes.
    async fn discovery(&mut self, index: usize) -> Result<Step> {
        self.best_effort(DISCOVERY[index]).await;
        Ok(next(
            DISCOVERY,
            index,
            |i| Step::Discovery { index: i },
            Step::Authenticate { attempt: 0 },
        ))
    }

    /// Steps 5/6: normalize the address, then create it, regenerating the
    /// address while it is already taken or rejected as invalid.
    async fn authenticate(&mut self, attempt: u8) -> Result<Step> {
        anyhow::ensure!(
            attempt < MAX_AUTH_ATTEMPTS,
            "/auth produced no response after {MAX_AUTH_ATTEMPTS} attempts"
        );

        let normalized: Value = self
            .post_json(URL_NORMALIZE, json!({ "email": self.account.email }))
            .await?
            .json()
            .await?;

        if normalized["exists"].as_bool() == Some(true) {
            self.account.regenerate_email();
            return Ok(Step::Authenticate {
                attempt: attempt + 1,
            });
        }

        let auth_body = json!({
            "email": self.account.email,
            "password": self.account.password,
            "homePostalCode": self.session.postal,
            "terms": false,
            "anonymous_id": self.session.device,
        });

        match self.post_json(URL_AUTH, auth_body).await {
            Ok(resp) => {
                self.auth = resp.json().await?;
                Ok(Step::ConfirmUser)
            }
            Err(error) if error.to_string().contains("email address is not valid") => {
                if attempt + 1 >= MAX_AUTH_ATTEMPTS {
                    return Err(error);
                }
                self.account.regenerate_email();
                Ok(Step::Authenticate {
                    attempt: attempt + 1,
                })
            }
            Err(error) => Err(error),
        }
    }

    /// Step 7: capture the tokens, then confirm the session against `/user`.
    async fn confirm_user(&mut self) -> Result<Step> {
        self.capture_credentials()?;

        let user: Value = self.get(URL_USER, headers::api_full).await?.json().await?;
        let expected = self.credentials.as_ref().map(|c| c.email.as_str());
        anyhow::ensure!(
            user["data"]["email"].as_str() == expected,
            "/user did not return the created email"
        );
        Ok(Step::Warmup { index: 0 })
    }

    /// Step 8: authenticated warmup.
    async fn warmup(&mut self, index: usize) -> Result<Step> {
        self.best_effort(WARMUP[index]).await;
        Ok(next(
            WARMUP,
            index,
            |i| Step::Warmup { index: i },
            Step::Done,
        ))
    }

    // -- completion --------------------------------------------------------

    /// Extracts tokens and identity from the `/auth` payload.
    fn capture_credentials(&mut self) -> Result<()> {
        let data = &self.auth["data"];
        let tokens = &data["authTokens"];

        let email = data["email"]
            .as_str()
            .unwrap_or(self.account.email.as_str())
            .to_owned();
        let access_token = tokens["access_token"]
            .as_str()
            .context("/auth returned no access_token")?
            .to_owned();
        let refresh_token = tokens["refresh_token"]
            .as_str()
            .context("/auth returned no refresh_token")?
            .to_owned();

        self.account.email = email.clone();
        self.session.user_id = data["id"].as_str().map(str::to_owned);
        self.session.access_token = Some(access_token.clone());
        self.credentials = Some(Credentials {
            email,
            access_token,
            refresh_token,
            expires_in: tokens["expires_in"].as_i64().unwrap_or_default(),
        });
        Ok(())
    }

    /// Consumes the flow into the public result.
    fn finish(self) -> Result<CreatedAccount> {
        let credentials = self.credentials.context("/auth produced no credentials")?;
        Ok(CreatedAccount {
            email: self.account.email.clone(),
            password: self.account.password.clone(),
            user_id: self.session.user_id.clone().unwrap_or_default(),
            device_id: self.session.device.clone(),
            access_token: credentials.access_token,
            refresh_token: credentials.refresh_token,
            expires_in: credentials.expires_in,
            created_at: chrono::Utc::now().to_rfc3339(),
        })
    }
}

/// Builds the next hit step via `more`, or `after` once `hits` is exhausted.
fn next(hits: &[Noise], index: usize, more: impl Fn(usize) -> Step, after: Step) -> Step {
    if index + 1 < hits.len() {
        more(index + 1)
    } else {
        after
    }
}

/// Runs the full signup flow, returning the created account credentials.
///
/// `account` may have its email regenerated if the address is already taken.
pub async fn create_account(
    client: &wreq::Client,
    account: &mut Account,
) -> Result<CreatedAccount> {
    let mut flow = Flow::new(client, account);
    let mut step = Step::Welcome;

    while step != Step::Done {
        step = flow.advance(step).await?;
    }

    flow.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_walks_the_group_then_hands_over() {
        let hits = &[Noise::Home, Noise::Popular];
        let more = |i| Step::Analytics { index: i };
        assert_eq!(
            next(hits, 0, more, Step::Done),
            Step::Analytics { index: 1 }
        );
        assert_eq!(next(hits, 1, more, Step::Done), Step::Done);
    }

    fn ctx() -> SessionCtx<'static> {
        SessionCtx {
            device: "d",
            session_id: "s",
            ad_id: "a",
            referer: headers::REFERER,
            postal: "32136",
            country: "USA",
            access_token: None,
            user_id: None,
            cookie_hdr: None,
            body_kind: BodyKind::None,
        }
    }

    #[test]
    fn noise_urls_target_the_right_hosts() {
        assert_eq!(Noise::Home.url(), URL_HOME);
        assert!(Noise::GtmSetCookie.url().starts_with(URL_GTM_SET_COOKIE));
        assert!(Noise::Tatari.url().starts_with(URL_TATARI));
    }

    #[test]
    fn noise_picks_the_document_builder_only_for_home() {
        let ctx = ctx();
        let doc = (Noise::Home.headers())(&ctx).unwrap();
        assert_eq!(doc["upgrade-insecure-requests"], "1");

        let api = (Noise::Popular.headers())(&ctx).unwrap();
        assert_eq!(api["x-postal-code"], "32136");
        assert!(api.get("upgrade-insecure-requests").is_none());
    }

    #[test]
    fn jar_scopes_by_domain_and_path() {
        let mut jar = CookieJar::default();
        let cookie = |name: &str, domain: &str, path: &str| Cookie {
            name: name.to_owned(),
            value: "v".to_owned(),
            domain: domain.to_owned(),
            path: path.to_owned(),
        };
        jar.entries.push(cookie("a", "fubo.tv", "/"));
        jar.entries.push(cookie("b", "other.tv", "/"));
        jar.entries.push(cookie("c", "fubo.tv", "/api"));

        let url = url::Url::parse("https://www.fubo.tv/signup").unwrap();
        assert_eq!(jar.header_for(&url).as_deref(), Some("a=v"));
        assert_eq!(jar.get("b").as_deref(), Some("v"));
    }
}
