//! Babel task audio staging: resolves remote `processedRecordingUriMap`
//! sources (`s3://` presigned per track with `transcriptions.getPresignedChunkUrls`, as
//! Babel's editor does, or plain `http(s)://`), downloads them into a local cache,
//! validates them as PCM16 WAV, and rewrites the payload to point at the local files.

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;

use crate::babel::{BabelConfig, presigned_chunk_urls_payload};
use studio_services::http;

/// A Babel payload whose audio sources are local files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedPayload {
    /// Payload JSON with remote audio URIs replaced by local paths.
    pub source: String,
    /// Base directory for resolving lane audio (`.` when nothing was downloaded).
    pub audio_base_dir: PathBuf,
    /// Remote audio references found in the payload.
    pub remote_audio_count: usize,
    /// Distinct files staged (downloaded or reused from cache).
    pub prepared_audio_count: usize,
}

impl PreparedPayload {
    /// A payload that needs no download.
    pub fn local(source: String) -> Self {
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

/// True when the payload references any `s3://` or `http(s)://` audio.
pub fn payload_needs_audio(source: &str) -> bool {
    serde_json::from_str::<Value>(source)
        .map(|value| !collect_remote_audio_sources(&value).is_empty())
        .unwrap_or(false)
}

/// Downloads every remote audio source into `<audio cache>/<actionId>/` (reusing
/// valid cached files) and rewrites the payload. Fails if any download fails or
/// a file is not PCM16 WAV. `config` supplies the Babel URL/auth for presigning.
pub async fn prepare_payload_audio(
    source: String,
    config: &BabelConfig,
) -> Result<PreparedPayload> {
    let mut value = serde_json::from_str::<Value>(&source)
        .context("failed to decode Babel payload for audio download")?;
    let remote_sources = collect_remote_audio_sources(&value);
    if remote_sources.is_empty() {
        return Ok(PreparedPayload::local(source));
    }

    // `processedRecordingUriMap` is keyed by `chunkedProcessedRecordingId`; Babel presigns
    // by those ids for the claimed review action.
    let action_id =
        first_string_by_key(&value, "actionId").unwrap_or_else(|| "unknown-action".to_string());
    let s3_recording_ids = remote_sources
        .iter()
        .filter(|source| is_s3_audio_source(&source.source_url))
        .map(|source| source.recording_key.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let presigned_urls = fetch_presigned_urls(config, &action_id, &s3_recording_ids).await?;
    let cache_dir = audio_cache_dir(config).join(sanitize_cache_component(&action_id));
    fs::create_dir_all(&cache_dir)
        .with_context(|| format!("failed to create Babel audio cache {}", cache_dir.display()))?;

    let client = http::client();
    let mut replacements = BTreeMap::new();
    for source in &remote_sources {
        if replacements.contains_key(&source.source_url) {
            continue;
        }
        let download_url = download_url_for_remote_audio(source, &presigned_urls)?;
        let local_path = cache_dir.join(cache_file_name(source));
        download_audio_file(&client, &download_url, &local_path).await?;
        replacements.insert(
            source.source_url.clone(),
            local_path.to_string_lossy().to_string(),
        );
    }

    if rewrite_remote_audio_sources(&mut value, &replacements) == 0 {
        bail!("Babel audio download finished, but payload audio paths were not rewritten");
    }
    Ok(PreparedPayload {
        source: value.to_string(),
        audio_base_dir: cache_dir,
        remote_audio_count: remote_sources.len(),
        prepared_audio_count: replacements.len(),
    })
}

/// Presigned HTTPS URLs for a review action, using its explicit direct/relay origin.
pub async fn fetch_presigned_urls(
    config: &BabelConfig,
    review_action_id: &str,
    chunked_processed_recording_ids: &[String],
) -> Result<BTreeMap<String, String>> {
    if chunked_processed_recording_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    if let Some(session) = &config.relay_session {
        return crate::subaccounts::audio_presigned_urls(
            session,
            review_action_id,
            chunked_processed_recording_ids,
        )
        .await;
    }
    let body =
        presigned_chunk_urls_payload(config, review_action_id, chunked_processed_recording_ids)
            .await
            .map_err(|error| anyhow!("Babel presigned audio URL request failed: {error}"))?;
    parse_presigned_url_response(&body)
}

fn parse_presigned_url_response(body: &str) -> Result<BTreeMap<String, String>> {
    let value = serde_json::from_str::<Value>(body)
        .context("failed to decode Babel presigned audio URL response")?;
    let data = trpc_json_payload(&value)
        .ok_or_else(|| anyhow!("Babel presigned audio URL response did not include data"))?;
    let map = data
        .as_object()
        .ok_or_else(|| anyhow!("Babel presigned audio URL response was not an object"))?;
    Ok(map
        .iter()
        .filter_map(|(source, signed)| {
            signed
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|signed| (source.clone(), signed.to_string()))
        })
        .collect())
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

/// `<account dir>/audio-cache` (the config's `data_dir`, else the auth store's directory),
/// or `<temp>/TranscriptionStudio/audio-cache`.
pub fn audio_cache_dir(config: &BabelConfig) -> PathBuf {
    let account_dir = config.data_dir.clone().or_else(|| {
        config
            .auth_store
            .as_deref()
            .and_then(Path::parent)
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
    });
    match account_dir {
        Some(dir) => dir.join("audio-cache"),
        None => env::temp_dir()
            .join("TranscriptionStudio")
            .join("audio-cache"),
    }
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
                    if let Some(local_path) = source
                        .as_str()
                        .and_then(|source_url| replacements.get(source_url.trim()))
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

fn download_url_for_remote_audio(
    source: &RemoteAudioSource,
    presigned_urls: &BTreeMap<String, String>,
) -> Result<String> {
    if is_http_audio_source(&source.source_url) {
        return Ok(source.source_url.clone());
    }
    presigned_urls
        .get(&source.recording_key)
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or_else(|| {
            anyhow!(
                "Babel did not return a presigned URL for track {}",
                source.recording_key
            )
        })
}

async fn download_audio_file(
    client: &reqwest::Client,
    download_url: &str,
    local_path: &Path,
) -> Result<()> {
    if local_audio_file_is_usable(local_path) {
        return Ok(());
    }

    let response = client
        .get(download_url)
        .send()
        .await
        .context("Babel audio download failed")?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .context("failed to read Babel audio download")?;
    if !status.is_success() {
        bail!(
            "Babel audio download HTTP {status}: {}",
            http::body_snippet(&String::from_utf8_lossy(&bytes))
        );
    }
    studio_waveform::decode_pcm16_wav(&bytes).map_err(|error| {
        anyhow!("downloaded Babel audio is not a supported PCM16 WAV: {error:?}")
    })?;
    if let Some(parent) = local_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create Babel audio cache {}", parent.display()))?;
    }
    fs::write(local_path, &bytes)
        .with_context(|| format!("failed to write Babel audio cache {}", local_path.display()))
}

fn local_audio_file_is_usable(path: &Path) -> bool {
    fs::read(path).is_ok_and(|bytes| studio_waveform::decode_pcm16_wav(&bytes).is_ok())
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

fn cache_file_name(source: &RemoteAudioSource) -> String {
    format!(
        "{}.{}",
        sanitize_cache_component(&source.recording_key),
        audio_extension(&source.source_url)
    )
}

fn audio_extension(source: &str) -> String {
    let path = url::Url::parse(source)
        .map(|url| url.path().to_string())
        .unwrap_or_else(|_| source.to_string());
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

fn first_string_by_key(value: &Value, key: &str) -> Option<String> {
    match value {
        Value::Object(map) => map
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                map.values()
                    .find_map(|child| first_string_by_key(child, key))
            }),
        Value::Array(items) => items
            .iter()
            .find_map(|child| first_string_by_key(child, key)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn detects_remote_review_action_audio_sources() {
        let source = r#"{"result":{"data":{"json":{
            "actionId": "action-1",
            "processedRecordingUriMap": {
                "recording-1": "s3://bucket/a.wav",
                "recording-2": "audio/speaker_2.wav"
            }
        }}}}"#;
        assert!(payload_needs_audio(source));
        assert!(!payload_needs_audio(
            r#"{"processedRecordingUriMap":{"r":"audio/a.wav"}}"#
        ));
    }

    #[test]
    fn rewrites_remote_audio_sources_from_presigned_downloads() {
        let mut value = json!({"result": {"data": {"json": {"processedRecordingUriMap": {
            "recording-1": "s3://bucket/a.wav",
            "recording-2": "audio/speaker_2.wav"
        }}}}});
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
            Some(&json!("C:\\cache\\recording-1.wav"))
        );
        assert_eq!(
            value.pointer("/result/data/json/processedRecordingUriMap/recording-2"),
            Some(&json!("audio/speaker_2.wav"))
        );
    }

    #[test]
    fn presigned_url_response_parses_trpc_batch_shape() {
        let urls = parse_presigned_url_response(
            r#"[{"result":{"data":{"json":{"rec-a":"https://signed/a","rec-c":""}}}}]"#,
        )
        .expect("presigned urls");
        assert_eq!(
            urls,
            BTreeMap::from([("rec-a".to_string(), "https://signed/a".to_string())])
        );
        let source = |recording_key: &str, source_url: &str| RemoteAudioSource {
            recording_key: recording_key.to_string(),
            source_url: source_url.to_string(),
        };
        assert_eq!(
            download_url_for_remote_audio(&source("rec-a", "s3://b/a.wav"), &urls)
                .expect("presigned"),
            "https://signed/a"
        );
        assert!(download_url_for_remote_audio(&source("rec-c", "s3://b/c.wav"), &urls).is_err());
        assert_eq!(
            download_url_for_remote_audio(&source("rec-x", "https://cdn/x.wav"), &urls)
                .expect("http passthrough"),
            "https://cdn/x.wav"
        );
    }

    #[test]
    fn cache_file_names_are_sanitized_with_audio_extension() {
        let source = RemoteAudioSource {
            recording_key: "rec/1:a".to_string(),
            source_url: "s3://bucket/path/file.WAV".to_string(),
        };
        assert_eq!(cache_file_name(&source), "rec_1_a.wav");
        let source = RemoteAudioSource {
            recording_key: "..".to_string(),
            source_url: "https://cdn/x?sig=1".to_string(),
        };
        assert_eq!(cache_file_name(&source), "audio.wav");
    }
}
