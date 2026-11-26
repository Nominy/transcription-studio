use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
};

use reqwest::header::{
    ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue,
};
use serde_json::{Value, json};
use time::OffsetDateTime;

use super::native_auth::{
    default_native_auth_store_path, mint_native_babel_authorization_from_store, native_clerk_url,
};
use super::native_auth_cli::native_babel_url;
use super::subaccounts::subaccount_audio_presigned_urls;
#[path = "chunk_audio.rs"]
mod chunk_audio;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BabelPayloadLoadKind {
    ReviewAction,
    LatestReviewAction,
    ClaimByProcessedTranscription,
    ClaimNextFromQueue,
}

impl BabelPayloadLoadKind {
    pub(super) fn success_status(self) -> &'static str {
        match self {
            Self::ReviewAction => "Review action loaded.",
            Self::LatestReviewAction => "Processed transcription loaded.",
            Self::ClaimByProcessedTranscription => "Stitched transcription claimed.",
            Self::ClaimNextFromQueue => "Queue claim loaded.",
        }
    }

    pub(super) fn failure_label(self) -> &'static str {
        match self {
            Self::ReviewAction => "Review action load",
            Self::LatestReviewAction => "Processed transcription load",
            Self::ClaimByProcessedTranscription => "Stitched transcription claim",
            Self::ClaimNextFromQueue => "Queue claim load",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedBabelAudioPayload {
    pub(super) source: String,
    pub(super) audio_base_dir: PathBuf,
    pub(super) remote_audio_count: usize,
    pub(super) prepared_audio_count: usize,
}

impl PreparedBabelAudioPayload {
    pub(super) fn local(source: String) -> Self {
        Self {
            source,
            audio_base_dir: PathBuf::from("."),
            remote_audio_count: 0,
            prepared_audio_count: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteAudioSource {
    recording_key: String,
    source_url: String,
}

pub(super) fn babel_payload_needs_audio_preparation(source: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(source) else {
        return false;
    };
    collect_remote_audio_sources(&value)
        .iter()
        .any(|source| is_remote_audio_source(&source.source_url))
}

pub(super) async fn prepare_babel_audio_payload(
    source: String,
) -> Result<PreparedBabelAudioPayload, String> {
    let mut value = serde_json::from_str::<Value>(&source)
        .map_err(|error| format!("failed to decode Babel payload for audio download: {error}"))?;
    let remote_sources = collect_remote_audio_sources(&value);
    if remote_sources.is_empty() {
        return Ok(PreparedBabelAudioPayload::local(source));
    }

    let s3_urls = remote_sources
        .iter()
        .filter(|source| is_s3_audio_source(&source.source_url))
        .map(|source| source.source_url.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let presigned_urls = if s3_urls.is_empty() {
        BTreeMap::new()
    } else if let Some(input) = chunk_audio::request(&value)? {
        let by_chunk = fetch_chunk_urls(input).await?;
        let mut urls = BTreeMap::new();
        for source in remote_sources
            .iter()
            .filter(|s| is_s3_audio_source(&s.source_url))
        {
            let signed = by_chunk.get(&source.recording_key).ok_or_else(|| {
                format!(
                    "Babel did not return an audio link for fragment {}. Reload the task.",
                    source.recording_key
                )
            })?;
            urls.insert(source.source_url.clone(), signed.clone());
        }
        urls
    } else {
        fetch_presigned_urls(s3_urls).await?
    };
    let cache_dir = default_audio_cache_dir().join(sanitize_cache_component(
        &first_string_by_key(&value, &["actionId"]).unwrap_or_else(|| "unknown-action".to_string()),
    ));
    fs::create_dir_all(&cache_dir).map_err(|error| {
        format!(
            "failed to create Babel audio cache {}: {error}",
            cache_dir.display()
        )
    })?;

    let client = reqwest::Client::new();
    let mut replacements = BTreeMap::new();
    let mut prepared_audio_count = 0;
    for source in &remote_sources {
        if replacements.contains_key(&source.source_url) {
            continue;
        }
        let download_url = download_url_for_remote_audio(&source.source_url, &presigned_urls)?;
        let local_path = cache_dir.join(cache_file_name(source));
        prepare_audio_file(&client, &download_url, &local_path).await?;
        replacements.insert(
            source.source_url.clone(),
            local_path.to_string_lossy().to_string(),
        );
        prepared_audio_count += 1;
    }

    let rewritten = rewrite_remote_audio_sources(&mut value, &replacements);
    if rewritten == 0 {
        return Err(
            "Babel audio download finished, but payload audio paths were not rewritten".to_string(),
        );
    }

    Ok(PreparedBabelAudioPayload {
        source: value.to_string(),
        audio_base_dir: cache_dir,
        remote_audio_count: remote_sources.len(),
        prepared_audio_count,
    })
}

fn collect_remote_audio_sources(value: &Value) -> Vec<RemoteAudioSource> {
    let mut sources = Vec::new();
    collect_remote_audio_sources_into(value, &mut sources);
    sources
}

fn collect_remote_audio_sources_into(value: &Value, sources: &mut Vec<RemoteAudioSource>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::Object(uri_map)) = map.get("processedRecordingUriMap") {
                for (recording_key, source) in uri_map {
                    if let Some(source_url) = source.as_str().map(str::trim)
                        && is_remote_audio_source(source_url)
                    {
                        sources.push(RemoteAudioSource {
                            recording_key: recording_key.clone(),
                            source_url: source_url.to_string(),
                        });
                    }
                }
            }
            for child in map.values() {
                collect_remote_audio_sources_into(child, sources);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_remote_audio_sources_into(child, sources);
            }
        }
        _ => {}
    }
}

fn rewrite_remote_audio_sources(
    value: &mut Value,
    replacements: &BTreeMap<String, String>,
) -> usize {
    match value {
        Value::Object(map) => {
            let mut rewritten = 0;
            if let Some(Value::Object(uri_map)) = map.get_mut("processedRecordingUriMap") {
                for source in uri_map.values_mut() {
                    if let Some(source_url) = source.as_str()
                        && let Some(local_path) = replacements.get(source_url)
                    {
                        *source = Value::String(local_path.clone());
                        rewritten += 1;
                    }
                }
            }
            rewritten
                + map
                    .values_mut()
                    .map(|child| rewrite_remote_audio_sources(child, replacements))
                    .sum::<usize>()
        }
        Value::Array(items) => items
            .iter_mut()
            .map(|child| rewrite_remote_audio_sources(child, replacements))
            .sum(),
        _ => 0,
    }
}

async fn fetch_chunk_urls(input: Value) -> Result<BTreeMap<String, String>, String> {
    if let Some(body) = super::subaccounts::subaccount_chunk_audio_payload(input.clone()).await? {
        return chunk_audio::parse(&body);
    }
    let authorization = mint_native_babel_authorization_from_store(
        &default_native_auth_store_path(),
        &native_babel_url(),
        &native_clerk_url(),
        OffsetDateTime::now_utc().unix_timestamp(),
    )
    .await?;
    let query = json!({"0":{"json":input}}).to_string();
    let encoded: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
    let response = super::http_client::shared_http_client()
        .get(format!(
            "{}/api/trpc/transcriptions.getPresignedChunkUrls?batch=1&input={encoded}",
            native_babel_url().trim_end_matches('/')
        ))
        .header(AUTHORIZATION, authorization)
        .header(ACCEPT_ENCODING, "identity")
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|_| "Audio link request failed. Try again.")?
        .error_for_status()
        .map_err(|e| {
            format!(
                "Audio link request failed: {}",
                e.status().map(|s| s.to_string()).unwrap_or_default()
            )
        })?;
    let body = response
        .text()
        .await
        .map_err(|_| "Audio link response was interrupted.")?;
    chunk_audio::parse(&body)
}

async fn fetch_presigned_urls(s3_urls: Vec<String>) -> Result<BTreeMap<String, String>, String> {
    if s3_urls.is_empty() {
        return Ok(BTreeMap::new());
    }

    if let Some(urls) = subaccount_audio_presigned_urls(&s3_urls).await? {
        return Ok(urls);
    }

    let babel_url = native_babel_url();
    let authorization = mint_native_babel_authorization_from_store(
        &default_native_auth_store_path(),
        &babel_url,
        &native_clerk_url(),
        OffsetDateTime::now_utc().unix_timestamp(),
    )
    .await?;
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&authorization)
            .map_err(|error| format!("Babel audio authorization header is invalid: {error}"))?,
    );

    let client = reqwest::Client::new();
    let mut presigned_urls = BTreeMap::new();
    for s3_url in s3_urls {
        let (bucket, key) = split_s3_url(&s3_url)?;
        let input = json!({
            "0": {
                "json": {
                    "bucket": bucket,
                    "key": key,
                }
            }
        })
        .to_string();
        let encoded_input: String =
            url::form_urlencoded::byte_serialize(input.as_bytes()).collect();
        let url = format!(
            "{}/api/trpc/application.getAudioPresignedUrl?batch=1&input={encoded_input}",
            babel_url.trim().trim_end_matches('/')
        );
        let response = client
            .get(url)
            .headers(headers.clone())
            .send()
            .await
            .map_err(|error| format!("Babel presigned audio URL request failed: {error}"))?;
        let status = response.status();
        let body = response.text().await.map_err(|error| {
            format!("failed to read Babel presigned audio URL response: {error}")
        })?;
        if !status.is_success() {
            return Err(format!(
                "Babel presigned audio URL HTTP {status}: {}",
                response_body_snippet(&body)
            ));
        }
        presigned_urls.insert(s3_url, parse_single_presigned_url_response(&body)?);
    }
    Ok(presigned_urls)
}

fn split_s3_url(source: &str) -> Result<(&str, &str), String> {
    let remainder = source
        .strip_prefix("s3://")
        .ok_or_else(|| format!("Babel audio source is not an S3 URL: {source}"))?;
    let (bucket, key) = remainder
        .split_once('/')
        .filter(|(bucket, key)| !bucket.is_empty() && !key.is_empty())
        .ok_or_else(|| format!("Babel audio S3 URL is invalid: {source}"))?;
    Ok((bucket, key))
}

fn parse_single_presigned_url_response(body: &str) -> Result<String, String> {
    let value = serde_json::from_str::<Value>(body)
        .map_err(|error| format!("failed to decode Babel presigned audio URL response: {error}"))?;
    let data = trpc_json_payload(&value)
        .ok_or_else(|| "Babel presigned audio URL response did not include data".to_string())?;
    data.as_str()
        .or_else(|| data.get("url").and_then(Value::as_str))
        .or_else(|| data.get("presignedUrl").and_then(Value::as_str))
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Babel presigned audio URL response did not include a URL".to_string())
}

fn trpc_json_payload(value: &Value) -> Option<&Value> {
    if let Value::Array(items) = value {
        return items.first().and_then(trpc_json_payload);
    }
    value
        .pointer("/result/data/json")
        .or_else(|| value.pointer("/data/json"))
        .or_else(|| value.pointer("/result/data"))
        .or_else(|| value.pointer("/data"))
}

fn download_url_for_remote_audio(
    source_url: &str,
    presigned_urls: &BTreeMap<String, String>,
) -> Result<String, String> {
    if is_http_audio_source(source_url) {
        return Ok(source_url.to_string());
    }
    presigned_urls
        .get(source_url)
        .cloned()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("Babel did not return a presigned URL for {source_url}"))
}

async fn prepare_audio_file(
    client: &reqwest::Client,
    download_url: &str,
    local_path: &Path,
) -> Result<(), String> {
    if local_audio_file_is_usable(local_path) {
        return Ok(());
    }

    let response = client
        .get(download_url)
        .send()
        .await
        .map_err(|error| format!("Babel audio download failed: {error}"))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("failed to read Babel audio download: {error}"))?;
    if !status.is_success() {
        let body = String::from_utf8_lossy(&bytes);
        return Err(format!(
            "Babel audio download HTTP {status}: {}",
            response_body_snippet(&body)
        ));
    }
    studio_waveform::decode_pcm16_wav(&bytes).map_err(|error| {
        format!("downloaded Babel audio is not a supported PCM16 WAV: {error:?}")
    })?;
    if let Some(parent) = local_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create Babel audio cache {}: {error}",
                parent.display()
            )
        })?;
    }
    fs::write(local_path, &bytes).map_err(|error| {
        format!(
            "failed to write Babel audio cache {}: {error}",
            local_path.display()
        )
    })
}

fn local_audio_file_is_usable(path: &Path) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    studio_waveform::decode_pcm16_wav(&bytes).is_ok()
}

fn is_remote_audio_source(source: &str) -> bool {
    is_s3_audio_source(source) || is_http_audio_source(source)
}

fn is_s3_audio_source(source: &str) -> bool {
    source.trim_start().starts_with("s3://")
}

fn is_http_audio_source(source: &str) -> bool {
    let source = source.trim_start();
    source.starts_with("https://") || source.starts_with("http://")
}

fn default_audio_cache_dir() -> PathBuf {
    if let Some(parent) = default_native_auth_store_path().parent() {
        return parent.join("audio-cache");
    }
    env::temp_dir()
        .join("TranscriptionStudio")
        .join("audio-cache")
}

fn cache_file_name(source: &RemoteAudioSource) -> String {
    format!(
        "{}.{}",
        sanitize_cache_component(&source.recording_key),
        audio_extension(&source.source_url)
    )
}

fn audio_extension(source: &str) -> String {
    let path = url::Url::parse(source)
        .ok()
        .map(|url| url.path().to_string())
        .unwrap_or_else(|| source.to_string());
    path.rsplit('/')
        .next()
        .and_then(|file_name| file_name.rsplit_once('.').map(|(_, extension)| extension))
        .map(|extension| sanitize_cache_component(extension).to_ascii_lowercase())
        .filter(|extension| {
            !extension.is_empty()
                && extension.len() <= 8
                && extension
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
        })
        .unwrap_or_else(|| "wav".to_string())
}

fn sanitize_cache_component(raw: &str) -> String {
    let sanitized = raw
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches(['.', '_'])
        .to_string();
    if sanitized.is_empty() {
        "audio".to_string()
    } else {
        sanitized
    }
}

fn first_string_by_key(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(Value::as_str) {
                    return Some(found.to_string());
                }
            }
            map.values()
                .find_map(|child| first_string_by_key(child, keys))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|child| first_string_by_key(child, keys)),
        _ => None,
    }
}

fn response_body_snippet(body: &str) -> String {
    let snippet: String = body
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(500)
        .collect();
    if snippet.is_empty() {
        "<empty response body>".to_string()
    } else {
        snippet
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_remote_review_action_audio_sources() {
        let source = r#"{
            "result": {
                "data": {
                    "json": {
                        "actionId": "action-1",
                        "processedRecordingUriMap": {
                            "recording-1": "s3://bucket/a.wav",
                            "recording-2": "audio/speaker_2.wav"
                        }
                    }
                }
            }
        }"#;

        assert!(babel_payload_needs_audio_preparation(source));
    }

    #[test]
    fn rewrites_remote_audio_sources_from_presigned_downloads() {
        let mut value = json!({
            "result": {
                "data": {
                    "json": {
                        "processedRecordingUriMap": {
                            "recording-1": "s3://bucket/a.wav",
                            "recording-2": "audio/speaker_2.wav"
                        }
                    }
                }
            }
        });
        let rewritten = rewrite_remote_audio_sources(
            &mut value,
            &BTreeMap::from([(
                "s3://bucket/a.wav".to_string(),
                "C:\\cache\\recording-1.wav".to_string(),
            )]),
        );

        assert_eq!(rewritten, 1);
        assert_eq!(
            value.pointer("/result/data/json/processedRecordingUriMap/recording-1"),
            Some(&Value::String("C:\\cache\\recording-1.wav".to_string()))
        );
        assert_eq!(
            value.pointer("/result/data/json/processedRecordingUriMap/recording-2"),
            Some(&Value::String("audio/speaker_2.wav".to_string()))
        );
    }

    #[test]
    fn parses_current_babel_single_audio_presign_contract() {
        assert_eq!(
            split_s3_url("s3://audio-bucket/path/to/recording.wav"),
            Ok(("audio-bucket", "path/to/recording.wav"))
        );
        assert_eq!(
            parse_single_presigned_url_response(
                r#"[{"result":{"data":{"json":"https://signed.example/audio.wav"}}}]"#,
            ),
            Ok("https://signed.example/audio.wav".to_string())
        );
    }
}
