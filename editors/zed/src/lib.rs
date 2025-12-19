use std::fs;
use zed_extension_api::{self as zed, Command, LanguageServerId, Result, Worktree};

struct SoppoExtension {
    cached_binary_path: Option<String>,
}

impl SoppoExtension {
    fn language_server_binary_path(
        &mut self,
        language_server_id: &LanguageServerId,
    ) -> Result<String> {
        if let Some(path) = &self.cached_binary_path {
            if fs::metadata(path).map_or(false, |stat| stat.is_file()) {
                return Ok(path.clone());
            }
        }

        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::CheckingForUpdate,
        );

        let release = self.find_latest_release()?;

        let (platform, arch) = zed::current_platform();
        let asset_name = format!(
            "sopls-{}-{}.tar.gz",
            match arch {
                zed::Architecture::Aarch64 => "aarch64",
                zed::Architecture::X8664 => "x86_64",
                _ => return Err("unsupported architecture".into()),
            },
            match platform {
                zed::Os::Mac => "apple-darwin",
                zed::Os::Linux => "unknown-linux-gnu",
                zed::Os::Windows => "pc-windows-msvc",
            }
        );

        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == asset_name)
            .ok_or_else(|| format!("no asset found matching {:?}", asset_name))?;

        let version_dir = format!("sopls-{}", release.version);
        let binary_path = format!("{version_dir}/sopls");

        if !fs::metadata(&binary_path).map_or(false, |stat| stat.is_file()) {
            zed::set_language_server_installation_status(
                language_server_id,
                &zed::LanguageServerInstallationStatus::Downloading,
            );

            zed::download_file(
                &asset.download_url,
                &version_dir,
                zed::DownloadedFileType::GzipTar,
            )
            .map_err(|e| format!("failed to download file: {e}"))?;

            zed::make_file_executable(&binary_path)?;
        }

        self.cached_binary_path = Some(binary_path.clone());
        Ok(binary_path)
    }

    fn find_latest_release(&self) -> Result<zed::GithubRelease> {
        let response = zed::http_client::fetch(&zed::http_client::HttpRequest {
            url: "https://api.github.com/repos/halcyonnouveau/soppo/releases".to_string(),
            method: zed::http_client::HttpMethod::Get,
            headers: vec![("User-Agent".to_string(), "zed-soppo-extension".to_string())],
            body: None,
            redirect_policy: zed::http_client::RedirectPolicy::FollowAll,
        })?;

        let releases: Vec<GithubReleaseResponse> = serde_json::from_slice(&response.body)
            .map_err(|e| format!("failed to parse releases: {e}"))?;

        // Find the latest v* release
        releases
            .into_iter()
            .find(|r| r.tag_name.starts_with('v') && !r.tag_name.starts_with("lsp-v"))
            .map(|r| zed::GithubRelease {
                version: r.tag_name.trim_start_matches('v').to_string(),
                assets: r
                    .assets
                    .into_iter()
                    .map(|a| zed::GithubReleaseAsset {
                        name: a.name,
                        download_url: a.browser_download_url,
                    })
                    .collect(),
            })
            .ok_or_else(|| "no v* release found".into())
    }
}

#[derive(serde::Deserialize)]
struct GithubReleaseResponse {
    tag_name: String,
    assets: Vec<GithubAssetResponse>,
}

#[derive(serde::Deserialize)]
struct GithubAssetResponse {
    name: String,
    browser_download_url: String,
}

impl zed::Extension for SoppoExtension {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        _worktree: &Worktree,
    ) -> Result<Command> {
        Ok(Command {
            command: self.language_server_binary_path(language_server_id)?,
            args: vec![],
            env: vec![],
        })
    }
}

zed::register_extension!(SoppoExtension);
