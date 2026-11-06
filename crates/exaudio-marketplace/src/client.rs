use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{Catalog, Error, Limits, Release, Result, catalog::plain_https, invalid};

const MAX_CATALOG_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cache {
    source: String,
    catalog: Catalog,
}

#[derive(Debug)]
pub struct CatalogSnapshot {
    pub catalog: Catalog,
    pub from_cache: bool,
    /// Failed online refresh is visible rather than masquerading as fresh data.
    pub refresh_error: Option<String>,
}

pub struct MarketplaceClient {
    http: reqwest::Client,
    source: String,
    cache: PathBuf,
    #[cfg(test)]
    fixtures: std::collections::BTreeMap<String, String>,
}

pub(crate) fn release_cdn(url: &Url) -> bool {
    plain_https(url)
        && match url.host_str() {
            Some("release-assets.githubusercontent.com") => {
                url.path().starts_with("/github-production-release-asset/")
            }
            Some("objects.githubusercontent.com") => {
                url.path().starts_with("/github-production-release-asset-")
            }
            _ => false,
        }
}

impl MarketplaceClient {
    /// `source` must be an explicit raw.githubusercontent.com index URL. The host chooses
    /// which reviewed index it trusts; no unofficial default is silently selected.
    pub fn new(source: &str, cache: impl Into<PathBuf>) -> Result<Self> {
        let url = Url::parse(source).map_err(|e| invalid(e.to_string()))?;
        let segments: Vec<_> = url.path_segments().into_iter().flatten().collect();
        if !plain_https(&url)
            || url.host_str() != Some("raw.githubusercontent.com")
            || url.query().is_some()
            || segments.len() < 4
            || segments.iter().any(|part| part.is_empty())
        {
            return Err(invalid(
                "catalog source must be an HTTPS raw.githubusercontent.com owner/repository/ref/path URL",
            ));
        }
        let http = reqwest::Client::builder()
            .user_agent("exaudio-marketplace/0.1")
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .redirect(Policy::custom(|attempt| {
                let release_origin = attempt
                    .previous()
                    .first()
                    .is_some_and(|url| plain_https(url) && url.host_str() == Some("github.com"));
                if attempt.previous().len() <= 4 && release_origin && release_cdn(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.error("redirect outside GitHub release CDN is forbidden")
                }
            }))
            .build()?;
        Ok(Self {
            http,
            source: source.to_owned(),
            cache: cache.into(),
            #[cfg(test)]
            fixtures: Default::default(),
        })
    }

    fn cached(&self) -> Result<Catalog> {
        let file = std::fs::File::open(&self.cache)?;
        if file.metadata()?.len() > MAX_CATALOG_BYTES + 4096 {
            return Err(invalid("oversized catalog cache"));
        }
        let cache: Cache = serde_json::from_reader(file)?;
        if cache.source != self.source {
            return Err(invalid("cache belongs to a different index"));
        }
        cache.catalog.validate()?;
        Ok(cache.catalog)
    }

    pub async fn catalog(&self, offline: bool) -> Result<CatalogSnapshot> {
        self.catalog_with_progress(offline, &crate::OperationControl::default())
            .await
    }

    pub async fn catalog_with_progress(
        &self,
        offline: bool,
        control: &crate::OperationControl,
    ) -> Result<CatalogSnapshot> {
        control.check()?;
        if offline {
            return Ok(CatalogSnapshot {
                catalog: self.cached()?,
                from_cache: true,
                refresh_error: None,
            });
        }
        let previous = self.cached().ok();
        let fetched = async {
            let bytes = self
                .fetch_controlled(&self.source, MAX_CATALOG_BYTES, None, control)
                .await?;
            let catalog = Catalog::parse(&bytes)?;
            if let Some(old) = &previous {
                old.validate_successor(&catalog)?;
            }
            control.check()?;
            atomic_json(
                &self.cache,
                &Cache {
                    source: self.source.clone(),
                    catalog: catalog.clone(),
                },
            )?;
            Ok::<_, Error>(catalog)
        }
        .await;
        match fetched {
            Ok(catalog) => Ok(CatalogSnapshot {
                catalog,
                from_cache: false,
                refresh_error: None,
            }),
            Err(Error::Cancelled) => Err(Error::Cancelled),
            Err(error) => match previous {
                Some(catalog) => Ok(CatalogSnapshot {
                    catalog,
                    from_cache: true,
                    refresh_error: Some(error.to_string()),
                }),
                None => Err(error),
            },
        }
    }

    /// The release must come from a validated catalog. Validation is repeated here for
    /// origin and bounds so callers cannot turn this into an arbitrary URL fetcher.
    pub async fn download(&self, release: &Release, limits: Limits) -> Result<Vec<u8>> {
        self.download_with_progress(release, limits, &crate::OperationControl::default())
            .await
    }

    pub async fn download_with_progress(
        &self,
        release: &Release,
        limits: Limits,
        control: &crate::OperationControl,
    ) -> Result<Vec<u8>> {
        control.check()?;
        let url = Url::parse(&release.asset.url).map_err(|e| invalid(e.to_string()))?;
        let parts: Vec<_> = url.path_segments().into_iter().flatten().collect();
        if !plain_https(&url)
            || url.host_str() != Some("github.com")
            || url.query().is_some()
            || parts.len() != 6
            || parts[2] != "releases"
            || parts[3] != "download"
            || parts[4] != release.tag
            || parts[5] != release.asset.name
            || !parts.iter().all(|part| crate::catalog::safe_token(part))
            || !crate::catalog::is_package_name(&release.asset.name)
        {
            return Err(invalid("invalid GitHub release asset URL"));
        }
        if release.withdrawn.is_some() {
            return Err(invalid("release is withdrawn"));
        }
        if release.asset.bytes == 0 || release.asset.bytes > limits.compressed_bytes {
            return Err(invalid("release exceeds compressed package limit"));
        }
        let bytes = self
            .fetch_controlled(
                &release.asset.url,
                release.asset.bytes,
                Some(&release.manifest.id),
                control,
            )
            .await?;
        crate::check_digest(&bytes, release, limits)?;
        Ok(bytes)
    }

    async fn fetch_controlled(
        &self,
        url: &str,
        maximum: u64,
        extension: Option<&str>,
        control: &crate::OperationControl,
    ) -> Result<Vec<u8>> {
        control.check()?;
        control.emit(
            crate::Stage::Downloading,
            extension,
            0,
            extension.map(|_| maximum),
        );
        #[cfg(test)]
        let url = self.fixtures.get(url).map_or(url, String::as_str);
        let response = tokio::select! {
            response = self.http.get(url).send() => response.map_err(|error| Error::Http(error.without_url()))?,
            _ = control.cancelled() => return Err(Error::Cancelled),
        };
        let mut response = response
            .error_for_status()
            .map_err(|error| Error::Http(error.without_url()))?;
        if response
            .content_length()
            .is_some_and(|length| length > maximum)
        {
            return Err(invalid("HTTP content exceeds byte limit"));
        }
        let total = response
            .content_length()
            .or_else(|| extension.map(|_| maximum));
        let mut bytes = Vec::new();
        loop {
            let chunk = tokio::select! {
                chunk = response.chunk() => chunk.map_err(|error| Error::Http(error.without_url()))?,
                _ = control.cancelled() => return Err(Error::Cancelled),
            };
            let Some(chunk) = chunk else {
                break;
            };
            control.check()?;
            if (bytes.len() as u64).saturating_add(chunk.len() as u64) > maximum {
                return Err(invalid("stream exceeds byte limit"));
            }
            bytes.extend_from_slice(&chunk);
            control.emit(
                crate::Stage::Downloading,
                extension,
                bytes.len() as u64,
                total,
            );
        }
        Ok(bytes)
    }

    #[cfg(test)]
    pub(crate) fn fixture(&mut self, original: &str, local: String) {
        let url = Url::parse(&local).unwrap();
        assert_eq!(url.scheme(), "http");
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        self.fixtures.insert(original.to_owned(), local);
    }
}

pub(crate) fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}
