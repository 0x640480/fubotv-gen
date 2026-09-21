//! Static client header literals and ordered [`HeaderMap`] builders.
//!
//! Every builder inserts headers in the exact order observed on the wire so
//! anti-bot fingerprint checks see a Chrome-identical header list.

use anyhow::Result;
use wreq::header::{HeaderMap, HeaderName, HeaderValue};

/// Origin used by the web client.
pub const TARGET_ORIGIN: &str = "https://www.fubo.tv";
/// Referer sent on API calls.
pub const REFERER: &str = "https://www.fubo.tv/";

/// Chrome user agent.
pub const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36";
/// `sec-ch-ua` client hint.
pub const SEC_CH_UA: &str =
    "\"Google Chrome\";v=\"153\", \"Not_A Brand\";v=\"8\", \"Chromium\";v=\"153\"";
/// `sec-ch-ua-mobile` client hint.
pub const SEC_CH_UA_MOBILE: &str = "?0";
/// `sec-ch-ua-platform` client hint.
pub const SEC_CH_UA_PLATFORM: &str = "\"Windows\"";

/// CORS wildcard accept.
pub const ACCEPT_JSON: &str = "*/*";
/// Document navigation accept.
pub const ACCEPT_DOC: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7";
/// Accept-Encoding.
pub const ACCEPT_ENCODING: &str = "gzip, deflate, br, zstd";
/// Accept-Language.
pub const ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";

/// Client feature flag list.
pub const SUPPORTED_FEATURES: &str = "alternate_broadcasts,auto_play_up_next,bracket,braze_custom_event,card_bottom_scrim,card_score_overlay,catalog_header,commitment,custom_loader,folder_sort_options,heartbeat_cm,horizontal_menu,initial_focus_target,list_item_score_overlay,load_channels_in_guide,locked_cm_pdp_odp,migrate_messaging,mfa_v1,notification_squares,odp_v2,playback_template_v2,playback_triggers,post_authentication_flow_v2,preferences,premium_cards,reorder_favorites,server_side_event,scheduled_as_nav_entry,score_overlay_v2,simple_analytics,scrub_nonads_channel,tags_consolidated,unified_notifications,use_drm_v2_response,vidai_my_stuff_moments,vidai_timeline_markers";
/// Supported audio codecs.
pub const SUPPORTED_AUDIO_CODECS: &str =
    "MP3,Ogg Vorbis,Ogg Opus,WebM Vorbis,WebM Opus,AAC,FLAC,WAV (LPCM)";

/// Device model string.
pub const X_DEVICE_MODEL: &str = "Windows NT 10.0 Chrome 153.0.0.0";
/// Client version.
pub const X_CLIENT_VERSION: &str = "6.42.0";
/// Player version.
pub const X_PLAYER_VERSION: &str = "7.4.2";
/// Operating system.
pub const X_OS: &str = "Windows";
/// Operating system version.
pub const X_OS_VERSION: &str = "NT 10.0";
/// Browser name.
pub const X_BROWSER: &str = "Chrome";
/// Browser version.
pub const X_BROWSER_VERSION: &str = "153.0.0.0";
/// Browser engine.
pub const X_BROWSER_ENGINE: &str = "Blink";
/// Timezone offset in minutes.
pub const X_TZ_OFFSET: &str = "-240";
/// Preferred language.
pub const X_PREFERRED_LANGUAGE: &str = "en-US";
/// Screen width.
pub const X_SCREEN_WIDTH: &str = "1251";
/// Screen height.
pub const X_SCREEN_HEIGHT: &str = "1278";
/// Supported streaming protocols.
pub const X_STREAMING_PROTOCOLS: &str = "hls,dash";
/// Supported codecs.
pub const X_CODECS: &str = "avc";
/// Device group.
pub const X_DEVICE_GROUP: &str = "desktop";
/// Device app.
pub const X_DEVICE_APP: &str = "web";
/// Device platform.
pub const X_DEVICE_PLATFORM: &str = "desktop";
/// Device type.
pub const X_DEVICE_TYPE: &str = "desktop";
/// Application id.
pub const X_APPLICATION_ID: &str = "fubo";
/// DRM scheme.
pub const X_DRM_SCHEME: &str = "widevine";

/// Priority header on API calls.
pub const PRIORITY_API: &str = "u=1, i";
/// Priority header on document navigations.
pub const PRIORITY_DOC: &str = "u=0, i";

/// Response header carrying the server-computed device id.
pub const X_DEVICE_ID_COMPUTED: &str = "x-device-id-computed";

/// Whether a request carries a JSON body (controls `content-type` placement).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    /// No body.
    None,
    /// JSON body.
    Json,
}

/// Per-request dynamic value bundle threaded into the builders.
pub struct SessionCtx<'a> {
    /// Device id.
    pub device: &'a str,
    /// Client session id.
    pub session_id: &'a str,
    /// Advertising id.
    pub ad_id: &'a str,
    /// Referer.
    pub referer: &'a str,
    /// Home postal code.
    pub postal: &'a str,
    /// Country code.
    pub country: &'a str,
    /// Bearer access token, once known.
    pub access_token: Option<&'a str>,
    /// User id, once known.
    pub user_id: Option<&'a str>,
    /// Serialized cookie header for the target host.
    pub cookie_hdr: Option<&'a str>,
    /// Body kind.
    pub body_kind: BodyKind,
}

fn set(hm: &mut HeaderMap, name: &'static str, val: impl AsRef<str>) -> Result<()> {
    hm.insert(
        HeaderName::from_static(name),
        HeaderValue::from_str(val.as_ref())?,
    );
    Ok(())
}

fn sec_ch_ua(hm: &mut HeaderMap) -> Result<()> {
    set(hm, "sec-ch-ua", SEC_CH_UA)?;
    set(hm, "sec-ch-ua-mobile", SEC_CH_UA_MOBILE)?;
    set(hm, "sec-ch-ua-platform", SEC_CH_UA_PLATFORM)?;
    Ok(())
}

fn doc_tail(hm: &mut HeaderMap, ctx: &SessionCtx<'_>) -> Result<()> {
    set(hm, "accept-encoding", ACCEPT_ENCODING)?;
    set(hm, "accept-language", ACCEPT_LANGUAGE)?;
    if let Some(c) = ctx.cookie_hdr {
        hm.append(HeaderName::from_static("cookie"), HeaderValue::from_str(c)?);
    }
    set(hm, "priority", PRIORITY_DOC)?;
    Ok(())
}

fn api_tail(hm: &mut HeaderMap, ctx: &SessionCtx<'_>) -> Result<()> {
    set(hm, "accept", ACCEPT_JSON)?;
    set(hm, "origin", TARGET_ORIGIN)?;
    set(hm, "sec-fetch-site", "same-site")?;
    set(hm, "sec-fetch-mode", "cors")?;
    set(hm, "sec-fetch-dest", "empty")?;
    set(hm, "referer", ctx.referer)?;
    set(hm, "accept-encoding", ACCEPT_ENCODING)?;
    set(hm, "accept-language", ACCEPT_LANGUAGE)?;
    if let Some(c) = ctx.cookie_hdr {
        hm.append(HeaderName::from_static("cookie"), HeaderValue::from_str(c)?);
    }
    set(hm, "priority", PRIORITY_API)?;
    Ok(())
}

/// Headers for the `/welcome` document navigation.
pub fn doc_welcome(ctx: &SessionCtx<'_>) -> Result<HeaderMap> {
    let mut hm = HeaderMap::new();
    set(&mut hm, "upgrade-insecure-requests", "1")?;
    set(&mut hm, "user-agent", UA)?;
    set(&mut hm, "accept", ACCEPT_DOC)?;
    set(&mut hm, "sec-fetch-site", "none")?;
    set(&mut hm, "sec-fetch-mode", "navigate")?;
    set(&mut hm, "sec-fetch-user", "?1")?;
    set(&mut hm, "sec-fetch-dest", "document")?;
    sec_ch_ua(&mut hm)?;
    doc_tail(&mut hm, ctx)?;
    Ok(hm)
}

/// Headers for the `/signup` document navigation.
pub fn doc_signup(ctx: &SessionCtx<'_>) -> Result<HeaderMap> {
    let mut hm = HeaderMap::new();
    sec_ch_ua(&mut hm)?;
    set(&mut hm, "upgrade-insecure-requests", "1")?;
    set(&mut hm, "user-agent", UA)?;
    set(&mut hm, "accept", ACCEPT_DOC)?;
    set(&mut hm, "sec-fetch-site", "same-origin")?;
    set(&mut hm, "sec-fetch-mode", "navigate")?;
    set(&mut hm, "sec-fetch-user", "?1")?;
    set(&mut hm, "sec-fetch-dest", "document")?;
    set(&mut hm, "referer", "https://www.fubo.tv/welcome")?;
    doc_tail(&mut hm, ctx)?;
    Ok(hm)
}

/// Headers for authenticated and long-form API calls.
pub fn api_full(ctx: &SessionCtx<'_>) -> Result<HeaderMap> {
    let mut hm = HeaderMap::new();
    set(&mut hm, "x-screen-width", X_SCREEN_WIDTH)?;
    set(&mut hm, "sec-ch-ua-platform", SEC_CH_UA_PLATFORM)?;
    if let Some(t) = ctx.access_token {
        set(&mut hm, "authorization", format!("Bearer {t}"))?;
    }
    if let Some(u) = ctx.user_id {
        set(&mut hm, "x-user-id", u)?;
    }
    set(&mut hm, "x-device-id", ctx.device)?;
    set(&mut hm, "x-screen-height", X_SCREEN_HEIGHT)?;
    set(&mut hm, "x-supported-features", SUPPORTED_FEATURES)?;
    set(&mut hm, "x-drm-scheme", X_DRM_SCHEME)?;
    sec_ch_ua(&mut hm)?;
    set(&mut hm, "x-device-app", X_DEVICE_APP)?;
    set(&mut hm, "x-player-version", X_PLAYER_VERSION)?;
    set(&mut hm, "x-device-platform", X_DEVICE_PLATFORM)?;
    set(&mut hm, "x-device-type", X_DEVICE_TYPE)?;
    set(&mut hm, "x-device-model", X_DEVICE_MODEL)?;
    if ctx.body_kind == BodyKind::Json {
        set(&mut hm, "content-type", "application/json")?;
    }
    set(&mut hm, "x-session-id", ctx.session_id)?;
    set(&mut hm, "x-application-id", X_APPLICATION_ID)?;
    set(
        &mut hm,
        "x-supported-streaming-protocols",
        X_STREAMING_PROTOCOLS,
    )?;
    set(&mut hm, "x-os-version", X_OS_VERSION)?;
    set(&mut hm, "x-device-group", X_DEVICE_GROUP)?;
    set(&mut hm, "x-supported-codecs-list", X_CODECS)?;
    set(&mut hm, "x-browser-engine", X_BROWSER_ENGINE)?;
    set(&mut hm, "x-ad-id", ctx.ad_id)?;
    set(&mut hm, "x-browser", X_BROWSER)?;
    set(&mut hm, "x-browser-version", X_BROWSER_VERSION)?;
    set(&mut hm, "x-supported-audio-codecs", SUPPORTED_AUDIO_CODECS)?;
    set(&mut hm, "x-preferred-language", X_PREFERRED_LANGUAGE)?;
    set(&mut hm, "user-agent", UA)?;
    set(&mut hm, "x-client-version", X_CLIENT_VERSION)?;
    set(&mut hm, "x-os", X_OS)?;
    set(&mut hm, "x-timezone-offset", X_TZ_OFFSET)?;
    api_tail(&mut hm, ctx)?;
    Ok(hm)
}

/// Headers for short-form unauthenticated API calls (`popular`, `has-free-trial`).
pub fn api_short(ctx: &SessionCtx<'_>) -> Result<HeaderMap> {
    let mut hm = HeaderMap::new();
    set(&mut hm, "sec-ch-ua-platform", SEC_CH_UA_PLATFORM)?;
    set(&mut hm, "x-device-id", ctx.device)?;
    set(&mut hm, "user-agent", UA)?;
    sec_ch_ua(&mut hm)?;
    set(&mut hm, "x-postal-code", ctx.postal)?;
    set(&mut hm, "x-country-code3", ctx.country)?;
    api_tail(&mut hm, ctx)?;
    Ok(hm)
}
