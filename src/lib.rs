use std::fs;
use zed::LanguageServerId;
use zed_extension_api::{self as zed, Result};

const BINARY_NAME: &str = "terraform-link-docs-lsp";
const REPO: &str = "laluowen/zed-terraform-link-docs";
const INSTALL_HINT: &str = "terraform-link-docs: could not find or install the language server \
`terraform-link-docs-lsp`. Install it by running `cargo install --path server` from the \
terraform-link-docs extension repository — this puts the binary on your PATH (usually ~/.cargo/bin).";

struct TerraformLinkDocsExtension {
    cached_binary_path: Option<String>,
}

impl TerraformLinkDocsExtension {
    fn language_server_binary_path(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<String> {
        // 1. Primary path: the user installed the server via `cargo install --path server`,
        //    landing it on PATH.
        if let Some(path) = worktree.which(BINARY_NAME) {
            return Ok(path);
        }

        // 2. A previously-downloaded binary that still exists.
        if let Some(path) = &self.cached_binary_path {
            if fs::metadata(path).is_ok_and(|stat| stat.is_file()) {
                return Ok(path.clone());
            }
        }

        // 3. Scaffold: attempt a GitHub release download. No releases exist yet, so this
        //    will error at runtime — we convert any failure into an actionable message.
        match self.download_binary(language_server_id) {
            Ok(path) => Ok(path),
            Err(err) => Err(format!("{INSTALL_HINT}\n(download attempt failed: {err})")),
        }
    }

    fn download_binary(&mut self, language_server_id: &LanguageServerId) -> Result<String> {
        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::CheckingForUpdate,
        );
        let release = zed::latest_github_release(
            REPO,
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;

        let (platform, arch) = zed::current_platform();
        let os = match platform {
            zed::Os::Mac => "mac",
            zed::Os::Linux => "linux",
            zed::Os::Windows => "windows",
        };
        let arch = match arch {
            zed::Architecture::Aarch64 => "aarch64",
            zed::Architecture::X86 => "x86",
            zed::Architecture::X8664 => "x86_64",
        };

        let asset_name = format!("{BINARY_NAME}-{arch}-{os}.zip");
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == asset_name)
            .ok_or_else(|| format!("no release asset matching `{asset_name}`"))?;

        let version_dir = format!("{BINARY_NAME}-{}", release.version);
        let binary_path = format!(
            "{version_dir}/{BINARY_NAME}{extension}",
            extension = match platform {
                zed::Os::Mac | zed::Os::Linux => "",
                zed::Os::Windows => ".exe",
            },
        );

        if !fs::metadata(&binary_path).is_ok_and(|stat| stat.is_file()) {
            zed::set_language_server_installation_status(
                language_server_id,
                &zed::LanguageServerInstallationStatus::Downloading,
            );

            zed::download_file(
                &asset.download_url,
                &version_dir,
                zed::DownloadedFileType::Zip,
            )
            .map_err(|e| format!("failed to download file: {e}"))?;

            zed::make_file_executable(&binary_path)?;
        }

        self.cached_binary_path = Some(binary_path.clone());
        Ok(binary_path)
    }
}

impl zed::Extension for TerraformLinkDocsExtension {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        Ok(zed::Command {
            command: self.language_server_binary_path(language_server_id, worktree)?,
            args: vec![],
            env: Default::default(),
        })
    }
}

zed::register_extension!(TerraformLinkDocsExtension);
