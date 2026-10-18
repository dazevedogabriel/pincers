//! Initial server probe: discovers size, range support, validators and file name.
//!
//! Like IDM's first connection, the probe is a real `GET` with `Range: bytes=0-`
//! (not a HEAD, which many servers answer differently). A `206` with a
//! `Content-Range` proves the server can serve byte ranges; the probe's body is
//! then handed to the first worker so no bytes are wasted.

use anyhow::{Context, Result, bail};
use percent_encoding::percent_decode_str;
use reqwest::{Client, Response, StatusCode, header};
use url::Url;

#[derive(Debug, Clone)]
pub struct RemoteInfo {
    /// URL after redirects.
    pub final_url: Url,
    /// Total size in bytes, if the server told us.
    pub size: Option<u64>,
    /// Whether the server honoured the byte range request (HTTP 206).
    pub resumable: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub file_name: String,
}

/// Sends the first request and returns the metadata together with the open
/// response so its body can be consumed as the first segment.
pub async fn probe(client: &Client, url: &str, headers: &[(String, String)]) -> Result<(RemoteInfo, Response)> {
    let mut req = client.get(url).header(header::RANGE, "bytes=0-");
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req
        .send()
        .await
        .with_context(|| crate::tr!("error.connect", url = url))?;

    let status = resp.status();
    if !(status == StatusCode::OK || status == StatusCode::PARTIAL_CONTENT) {
        bail!(crate::tr!("error.status", status = status));
    }

    let h = resp.headers();
    let text = |name: header::HeaderName| h.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);

    let (resumable, size) = if status == StatusCode::PARTIAL_CONTENT {
        // Content-Range: bytes 0-1023/4096
        let total = text(header::CONTENT_RANGE)
            .and_then(|cr| cr.rsplit('/').next().and_then(|t| t.trim().parse::<u64>().ok()));
        (total.is_some(), total)
    } else {
        (false, resp.content_length())
    };

    let final_url = resp.url().clone();
    let file_name = text(header::CONTENT_DISPOSITION)
        .and_then(|cd| file_name_from_disposition(&cd))
        .unwrap_or_else(|| file_name_from_url(&final_url));

    let info = RemoteInfo {
        final_url,
        size,
        resumable,
        etag: text(header::ETAG),
        last_modified: text(header::LAST_MODIFIED),
        file_name: sanitize(&file_name),
    };
    Ok((info, resp))
}

fn file_name_from_disposition(cd: &str) -> Option<String> {
    // RFC 6266: prefer filename*=UTF-8''..., fall back to filename="..."
    for part in cd.split(';').map(str::trim) {
        if let Some(v) = part.strip_prefix("filename*=") {
            let v = v.splitn(3, '\'').nth(2).unwrap_or(v);
            return Some(percent_decode_str(v).decode_utf8_lossy().into_owned());
        }
    }
    for part in cd.split(';').map(str::trim) {
        if let Some(v) = part.strip_prefix("filename=") {
            return Some(v.trim_matches('"').to_owned());
        }
    }
    None
}

fn file_name_from_url(url: &Url) -> String {
    url.path_segments()
        .and_then(|mut s| s.next_back())
        .filter(|s| !s.is_empty())
        .map(|s| percent_decode_str(s).decode_utf8_lossy().into_owned())
        .unwrap_or_else(|| "download.bin".to_owned())
}

/// Removes characters Windows does not allow in file names.
pub fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').to_owned();
    if cleaned.is_empty() { "download.bin".to_owned() } else { cleaned }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disposition_variants() {
        assert_eq!(file_name_from_disposition("attachment; filename=\"a b.zip\"").unwrap(), "a b.zip");
        assert_eq!(
            file_name_from_disposition("attachment; filename=x; filename*=UTF-8''ma%C3%A7%C3%A3.txt").unwrap(),
            "maçã.txt"
        );
    }

    #[test]
    fn sanitizes() {
        assert_eq!(sanitize("a:b?.txt"), "a_b_.txt");
        assert_eq!(sanitize("  "), "download.bin");
    }
}
