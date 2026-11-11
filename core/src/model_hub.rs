//! Discover GGUFs outside the starter catalog. Hugging Face revisions are
//! resolved to immutable commits before downloading; other hosts use HTTP
//! validators for resume and may optionally supply a SHA-256 checksum.

use anyhow::{bail, Context, Result};
use futures::StreamExt;
use reqwest::{header, Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Component, Path};
use std::time::Duration;

const MAX_CUSTOM_MODEL_BYTES: u64 = 128 * 1024 * 1024 * 1024;
const MAX_CUSTOM_MODEL_FILES: usize = 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelSource {
    HuggingFace {
        repository: String,
        filename: String,
        #[serde(default = "main_revision")]
        revision: String,
    },
    Url {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
    },
}

fn main_revision() -> String {
    "main".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadFile {
    pub filename: String,
    pub url: String,
    pub bytes: u64,
    pub sha256: Option<String>,
    /// Strong ETag or Last-Modified, suitable for If-Range. Never a size guess.
    pub validator: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteModel {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub bytes: u64,
    pub license: String,
    pub source: ModelSource,
    pub files: Vec<DownloadFile>,
}

impl RemoteModel {
    fn new(source: ModelSource, files: Vec<DownloadFile>, license: String) -> Result<Self> {
        let first = files.first().context("no GGUF files found")?;
        let display_name = crate::gguf::split_name(&first.filename)
            .map(|(prefix, _, _)| prefix.to_string())
            .unwrap_or_else(|| first.filename.trim_end_matches(".gguf").to_string());
        let description = match &source {
            ModelSource::HuggingFace { repository, .. } => repository.clone(),
            ModelSource::Url { url, .. } => http_url(url)?.host_str().unwrap_or("").to_string(),
        };
        // Include validators, so a changed URL artifact gets its own directory.
        let identity = serde_json::to_vec(&(&source, &files))?;
        let digest = format!("{:x}", Sha256::digest(identity));
        let model = Self {
            id: format!("custom-{}", &digest[..24]),
            display_name,
            description,
            bytes: files.iter().try_fold(0u64, |total, file| {
                total.checked_add(file.bytes).context("model size overflow")
            })?,
            license,
            source,
            files,
        };
        model.validate()?;
        Ok(model)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if !valid_custom_id(&self.id)
            || self.files.is_empty()
            || self.files.len() > MAX_CUSTOM_MODEL_FILES
        {
            bail!("invalid custom model record");
        }
        let mut names = std::collections::HashSet::new();
        let mut bytes = 0u64;
        for file in &self.files {
            validate_filename(&file.filename)?;
            http_url(&file.url)?;
            if file.bytes == 0 || !names.insert(&file.filename) {
                bail!("invalid or duplicate GGUF file");
            }
            if let Some(sha) = &file.sha256 {
                validate_sha256(sha)?;
            }
            bytes = bytes
                .checked_add(file.bytes)
                .context("model size overflow")?;
        }
        if bytes != self.bytes {
            bail!("custom model size does not match its files");
        }
        if bytes > MAX_CUSTOM_MODEL_BYTES {
            bail!("custom model exceeds the 128 GiB download limit");
        }
        Ok(())
    }
}

pub(crate) fn valid_custom_id(id: &str) -> bool {
    id.strip_prefix("custom-")
        .is_some_and(|suffix| suffix.len() == 24 && suffix.bytes().all(|b| b.is_ascii_hexdigit()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubRepository {
    pub id: String,
    pub downloads: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HubResults {
    pub repositories: Vec<HubRepository>,
    pub models: Vec<RemoteModel>,
    pub repository: Option<String>,
}

pub struct ModelHub {
    client: Client,
    base: Url,
}

impl ModelHub {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            base: Url::parse("https://huggingface.co/").unwrap(),
        }
    }

    /// A search phrase, owner/repo, repository page, or direct GGUF link.
    pub async fn browse(&self, query: &str) -> Result<HubResults> {
        let query = query.trim();
        if query.len() > 4096 {
            bail!("search or URL is too long");
        }
        if query.starts_with("http://") || query.starts_with("https://") {
            let url = http_url(query)?;
            if is_hugging_face(&url) {
                let parts = decoded_segments(&url)?;
                if parts.len() == 2 {
                    return self.repository(&parts.join("/"), "main").await;
                }
            }
            let model = self
                .resolve(&ModelSource::Url {
                    url: query.into(),
                    sha256: None,
                })
                .await?;
            return Ok(HubResults {
                models: vec![model],
                ..Default::default()
            });
        }
        if valid_repository(query) {
            return self.repository(query, "main").await;
        }
        let mut url = self.base.join("api/models")?;
        url.query_pairs_mut().extend_pairs([
            ("search", query),
            ("filter", "gguf"),
            ("sort", "downloads"),
            ("direction", "-1"),
            ("limit", "40"),
        ]);
        let value = self.json(url).await?;
        let repositories = value
            .as_array()
            .context("invalid model search response")?
            .iter()
            .filter_map(|value| {
                let id = value["id"].as_str()?;
                valid_repository(id).then(|| HubRepository {
                    id: id.into(),
                    downloads: value["downloads"].as_u64().unwrap_or(0),
                })
            })
            .collect();
        Ok(HubResults {
            repositories,
            ..Default::default()
        })
    }

    pub async fn resolve(&self, source: &ModelSource) -> Result<RemoteModel> {
        match source {
            ModelSource::HuggingFace {
                repository,
                filename,
                revision,
            } => {
                validate_filename(filename)?;
                self.repository(repository, revision)
                    .await?
                    .models
                    .into_iter()
                    .find(|model| model.files.iter().any(|file| file.filename == *filename))
                    .context(
                        "GGUF not found in this repository revision; choose a complete model file",
                    )
            }
            ModelSource::Url { url, sha256 } => {
                let url = http_url(url)?;
                if let Some(sha) = sha256 {
                    validate_sha256(sha)?;
                }
                if is_hugging_face(&url) {
                    let parts = decoded_segments(&url)?;
                    if parts.len() < 5 || !matches!(parts[2].as_str(), "resolve" | "blob") {
                        bail!("paste a Hugging Face GGUF file link or search owner/repository");
                    }
                    let repository = parts[..2].join("/");
                    let filename = parts[4..].join("/");
                    let mut model = self
                        .repository(&repository, &parts[3])
                        .await?
                        .models
                        .into_iter()
                        .find(|model| model.files.iter().any(|file| file.filename == filename))
                        .context("GGUF not found in this repository revision")?;
                    if let Some(expected) = sha256 {
                        let file = model
                            .files
                            .iter_mut()
                            .find(|file| file.filename == filename)
                            .unwrap();
                        if file
                            .sha256
                            .as_ref()
                            .is_some_and(|actual| !actual.eq_ignore_ascii_case(expected))
                        {
                            bail!("supplied checksum differs from the repository checksum");
                        }
                        file.sha256 = Some(expected.to_ascii_lowercase());
                    }
                    return Ok(model);
                }
                let filename = decoded_segments(&url)?
                    .last()
                    .cloned()
                    .context("URL has no filename")?;
                validate_filename(&filename)?;
                if crate::gguf::split_name(&filename).is_some() {
                    bail!("for a split GGUF, use its Hugging Face repository so every shard is downloaded");
                }
                let (bytes, validator) = self.url_metadata(url.clone()).await?;
                RemoteModel::new(
                    ModelSource::Url {
                        url: url.to_string(),
                        sha256: sha256.as_ref().map(|s| s.to_ascii_lowercase()),
                    },
                    vec![DownloadFile {
                        filename,
                        url: url.to_string(),
                        bytes,
                        sha256: sha256.as_ref().map(|s| s.to_ascii_lowercase()),
                        validator,
                    }],
                    String::new(),
                )
            }
        }
    }

    async fn repository(&self, repository: &str, revision: &str) -> Result<HubResults> {
        if !valid_repository(repository) || revision.is_empty() || revision.len() > 256 {
            bail!("enter a Hugging Face repository as owner/name");
        }
        let mut url = self.base.clone();
        url.path_segments_mut()
            .unwrap()
            .extend(["api", "models"])
            .extend(repository.split('/'))
            .extend(["revision", revision]);
        url.query_pairs_mut().append_pair("blobs", "true");
        self.repository_results(repository, &self.json(url).await?)
    }

    fn repository_results(&self, repository: &str, value: &Value) -> Result<HubResults> {
        let revision = value["sha"]
            .as_str()
            .context("repository has no commit revision")?;
        if revision.len() != 40 || !revision.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("repository returned an invalid commit revision");
        }
        let license = value["cardData"]["license"].as_str().unwrap_or("");
        let mut groups: BTreeMap<String, Vec<DownloadFile>> = BTreeMap::new();
        for sibling in value["siblings"]
            .as_array()
            .context("repository has no file list")?
        {
            let Some(filename) = sibling["rfilename"].as_str() else {
                continue;
            };
            if !filename.to_ascii_lowercase().ends_with(".gguf") {
                continue;
            }
            // Projectors/adapters cannot serve as a text model on their own.
            if Path::new(filename).file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .to_ascii_lowercase()
                    .starts_with("mmproj")
            }) {
                continue;
            }
            validate_filename(filename)?;
            let bytes = sibling["size"]
                .as_u64()
                .or_else(|| sibling["lfs"]["size"].as_u64())
                .context("repository did not return GGUF file sizes")?;
            let sha256 = sibling["lfs"]["sha256"]
                .as_str()
                .map(|s| s.to_ascii_lowercase());
            let mut url = self.base.clone();
            url.path_segments_mut()
                .unwrap()
                .extend(repository.split('/'))
                .extend(["resolve", revision])
                .extend(filename.split('/'));
            let group = crate::gguf::split_name(filename)
                .map(|(prefix, _, count)| format!("{prefix}:{count}"))
                .unwrap_or_else(|| filename.to_string());
            groups.entry(group).or_default().push(DownloadFile {
                filename: filename.into(),
                url: url.to_string(),
                bytes,
                sha256,
                validator: None,
            });
        }
        let mut models = Vec::new();
        for mut files in groups.into_values() {
            files.sort_by(|a, b| a.filename.cmp(&b.filename));
            if let Some((_, _, count)) = crate::gguf::split_name(&files[0].filename) {
                if count as usize > MAX_CUSTOM_MODEL_FILES
                    || files.len() != count as usize
                    || files.iter().enumerate().any(|(i, file)| {
                        crate::gguf::split_name(&file.filename)
                            .is_none_or(|(_, part, _)| part as usize != i + 1)
                    })
                {
                    continue;
                }
            }
            let source = ModelSource::HuggingFace {
                repository: repository.into(),
                filename: files[0].filename.clone(),
                revision: revision.into(),
            };
            models.push(RemoteModel::new(source, files, license.into())?);
        }
        models.sort_by_key(|model| model.bytes);
        Ok(HubResults {
            models,
            repository: Some(repository.into()),
            ..Default::default()
        })
    }

    async fn json(&self, url: Url) -> Result<Value> {
        let response = authenticated(self.client.get(url))
            .timeout(Duration::from_secs(30))
            .send()
            .await?;
        check_status(&response)?;
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if bytes.len().saturating_add(chunk.len()) > 16 * 1024 * 1024 {
                bail!("model listing is too large; open a more specific repository");
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).context("model host returned invalid JSON")
    }

    async fn url_metadata(&self, url: Url) -> Result<(u64, Option<String>)> {
        let client = direct_client()?;
        let response = client
            .head(url.clone())
            .timeout(Duration::from_secs(30))
            .send()
            .await?;
        if response.status().is_success() {
            if let Some(bytes) = response.content_length().filter(|bytes| *bytes > 0) {
                return Ok((bytes, http_validator(response.headers())));
            }
        }
        // Some file hosts do not implement HEAD. Drop the range response after
        // reading its headers; no model is buffered during browsing.
        let response = client
            .get(url)
            .header(header::RANGE, "bytes=0-0")
            .timeout(Duration::from_secs(30))
            .send()
            .await?;
        check_status(&response)?;
        let bytes = if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            response
                .headers()
                .get(header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.rsplit_once('/'))
                .and_then(|(_, total)| total.parse().ok())
        } else {
            response.content_length()
        };
        Ok((
            bytes
                .filter(|bytes| *bytes > 0)
                .context("file host must report Content-Length or a complete Content-Range")?,
            http_validator(response.headers()),
        ))
    }
}

pub(crate) fn authenticated(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    // reqwest drops Authorization when following a cross-host redirect. HF's
    // signed CDN links need no token; credentials never go to arbitrary hosts.
    let is_hf = request
        .try_clone()
        .and_then(|r| r.build().ok())
        .is_some_and(|r| is_hugging_face(r.url()));
    if is_hf {
        if let Ok(token) = std::env::var("HF_TOKEN") {
            if !token.trim().is_empty() {
                return request.bearer_auth(token.trim());
            }
        }
    }
    request
}

pub(crate) fn direct_client() -> Result<Client> {
    Ok(Client::builder()
        .user_agent(concat!("PrivateLM/", env!("CARGO_PKG_VERSION")))
        // Arbitrary direct links must not turn an apparently public host into a
        // request to a private service through an attacker-controlled redirect.
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(7200))
        .build()?)
}

fn check_status(response: &reqwest::Response) -> Result<()> {
    match response.status().as_u16() {
        200..=299 => Ok(()),
        401 | 403 => bail!("model host denied access; for gated Hugging Face models, accept access on Hugging Face and set HF_TOKEN on the coordinator"),
        404 => bail!("model or repository not found; check the repository and file link"),
        code => bail!("model host returned HTTP {code}; retry the search or download"),
    }
}

fn http_validator(headers: &header::HeaderMap) -> Option<String> {
    headers
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.starts_with("W/"))
        .or_else(|| {
            headers
                .get(header::LAST_MODIFIED)
                .and_then(|v| v.to_str().ok())
        })
        .map(String::from)
}

pub(crate) fn http_url(value: &str) -> Result<Url> {
    let url = Url::parse(value.trim()).context("enter a complete HTTP(S) GGUF link")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        bail!("use an HTTP(S) GGUF link without embedded credentials or a fragment");
    }
    Ok(url)
}

fn is_hugging_face(url: &Url) -> bool {
    url.host_str() == Some("huggingface.co")
}

fn decoded_segments(url: &Url) -> Result<Vec<String>> {
    url.path_segments()
        .context("URL has no path")?
        .filter(|part| !part.is_empty())
        .map(|part| {
            percent_encoding::percent_decode_str(part)
                .decode_utf8()
                .map(|s| s.into_owned())
                .context("invalid URL encoding")
        })
        .collect()
}

fn valid_repository(value: &str) -> bool {
    let parts: Vec<_> = value.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && !matches!(*part, "." | "..")
                && part.len() <= 256
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
}

pub(crate) fn validate_filename(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 1024
        || !value.to_ascii_lowercase().ends_with(".gguf")
        || value.chars().any(|c| c.is_control() || c == '\\')
        || value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || !Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        bail!("choose a GGUF model file with a valid relative filename");
    }
    Ok(())
}

fn validate_sha256(sha: &str) -> Result<()> {
    if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("SHA-256 must contain exactly 64 hexadecimal characters");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn repository_offers_all_quants_and_groups_complete_shards() {
        let hub = ModelHub::new(Client::new());
        let file =
            |name, size| json!({"rfilename":name,"size":size,"lfs":{"sha256":"a".repeat(64)}});
        let value = json!({"sha":"b".repeat(40),"cardData":{"license":"apache-2.0"},"siblings":[
            file("weights/Q4-00002-of-00002.gguf", 300),
            file("weights/Q4-00001-of-00002.gguf", 400),
            file("Q8.gguf", 900), file("incomplete-00001-of-00003.gguf", 100),
            file("mmproj-f16.gguf", 100), {"rfilename":"README.md"}
        ]});
        let result = hub.repository_results("someone/new-model", &value).unwrap();
        assert_eq!(result.models.len(), 2);
        assert_eq!(result.models[0].bytes, 700);
        assert_eq!(result.models[0].files.len(), 2);
        assert!(result.models[0].files[0]
            .filename
            .ends_with("00001-of-00002.gguf"));
        assert!(result.models[0].files[0]
            .url
            .contains(&format!("/resolve/{}/", "b".repeat(40))));
        assert_ne!(result.models[0].id, result.models[1].id);
        assert_eq!(result.models[1].license, "apache-2.0");
    }

    #[test]
    fn invalid_sources_cannot_escape_the_model_directory() {
        for name in [
            "../x.gguf",
            "/x.gguf",
            "foo/../x.gguf",
            "foo\\x.gguf",
            "x.safetensors",
            "x\n.gguf",
            "./x.gguf",
        ] {
            assert!(validate_filename(name).is_err(), "{name}");
        }
        assert!(validate_filename("quants/model.Q4_K_M.gguf").is_ok());
        for url in [
            "file:///tmp/x.gguf",
            "https://user:secret@example.com/x.gguf",
            "https://example.com/x.gguf#oops",
        ] {
            assert!(http_url(url).is_err());
        }
        assert!(!valid_custom_id("../../something"));
    }

    #[test]
    fn custom_models_have_bounded_disk_and_file_counts() {
        let file = DownloadFile {
            filename: "model.gguf".into(),
            url: "https://example.com/model.gguf".into(),
            bytes: MAX_CUSTOM_MODEL_BYTES + 1,
            sha256: None,
            validator: None,
        };
        assert!(RemoteModel::new(
            ModelSource::Url {
                url: file.url.clone(),
                sha256: None
            },
            vec![file],
            String::new(),
        )
        .is_err());
    }

    #[tokio::test]
    async fn arbitrary_direct_links_do_not_follow_redirects() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_url = format!("http://{}/secret.gguf", target.local_addr().unwrap());
        let redirect = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let redirect_url = format!("http://{}/model.gguf", redirect.local_addr().unwrap());
        let redirect_task = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = redirect.accept().await.unwrap();
                let mut request = [0; 2048];
                let _ = socket.read(&mut request).await.unwrap();
                socket.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {target_url}\r\nContent-Length: 0\r\n\r\n").as_bytes()).await.unwrap();
            }
        });
        let error = ModelHub::new(Client::new())
            .resolve(&ModelSource::Url {
                url: redirect_url,
                sha256: None,
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("HTTP 302"));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), target.accept())
                .await
                .is_err()
        );
        redirect_task.await.unwrap();
    }
}
