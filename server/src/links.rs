//! Computation of document links for Terraform resource/data blocks and module
//! `source` strings. Ported from the VSCode extension's
//! `src/terraform/resource.ts` and `src/terraform/module.ts`, hardcoded to
//! `registry.terraform.io`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use lsp_types::{DocumentLink, Position, Range, Url};
use regex::Regex;

fn resource_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"^(?P<prefix>\s*(?P<kind>resource|data)\s+")(?P<rtype>[^"]+)""#).unwrap()
    })
}

fn module_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"^(?P<prefix>\s*source\s+=\s+")(?P<src>[^"]+)""#).unwrap())
}

fn rtype_split_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(?P<provider>[^_]+)_(?P<name>.*)$").unwrap())
}

fn git_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^(?:git::(?:ssh://)?(([^@]+@)?))?(?:git::https?://)?(?:git@)?(?P<domain>[^:/]+)(?::|/)?(?P<repoPath>[^?#.]+)(?:\.git)?(?://(?P<module>[^?#]+))?(?:\?ref=(?P<ref>[^#]+))?(?:#.*)?$",
        )
        .unwrap()
    })
}

/// UTF-16 code-unit length of a string (LSP `character` offsets are UTF-16).
fn utf16_len(s: &str) -> u32 {
    s.encode_utf16().count() as u32
}

/// Build a link range on `line`, spanning from the end of `prefix` to the end
/// of `matched` (matching the VSCode port: `prefix.len()` .. `prefix.len() +
/// matched.len()`).
fn make_range(line: u32, prefix: &str, matched: &str) -> Range {
    let start = utf16_len(prefix);
    let end = start + utf16_len(matched);
    Range {
        start: Position { line, character: start },
        end: Position { line, character: end },
    }
}

/// Compute the documentation URL for a resource/data block.
///
/// `kind` is `"resource"` or `"data"`; `rtype` is e.g. `aws_s3_bucket`.
pub fn resource_url(
    kind: &str,
    rtype: &str,
    provider_map: &HashMap<String, String>,
) -> Option<String> {
    let caps = rtype_split_re().captures(rtype)?;
    let provider = caps.name("provider")?.as_str();
    let name = caps.name("name")?.as_str();
    if provider.is_empty() || name.is_empty() {
        return None;
    }

    let type_segment = if kind == "data" { "data-sources" } else { "resources" };

    // Known community provider source from the provider map.
    if let Some(source) = provider_map.get(provider) {
        let mut parts = source.splitn(2, '/');
        let namespace = parts.next().unwrap_or("");
        let provider_name = parts.next().unwrap_or("");
        return Some(format!(
            "https://registry.terraform.io/providers/{namespace}/{provider_name}/latest/docs/{type_segment}/{name}"
        ));
    }

    // Fallback: assume the hashicorp namespace.
    Some(format!(
        "https://registry.terraform.io/providers/hashicorp/{provider}/latest/docs/{type_segment}/{name}"
    ))
}

/// Compute the URL for a module `source` string. Returns `None` when no link
/// should be produced (raw http(s) sources, unsupported schemes, etc.).
pub fn module_url(src: &str, doc_path: Option<&Path>) -> Option<String> {
    if src.contains("http://") || src.contains("https://") {
        // The editor already handles bare URLs.
        None
    } else if src.starts_with('.') {
        local_path_url(src, doc_path)
    } else if src.starts_with("git::") || src.starts_with("github.com") || src.starts_with("git@github.com")
    {
        generic_git_url(src)
    } else if src.starts_with("gcs::") || src.starts_with("s3::") || src.starts_with("hg::") {
        // Suffix is already a URL; skip.
        None
    } else {
        registry_module_url(src)
    }
}

fn local_path_url(src: &str, doc_path: Option<&Path>) -> Option<String> {
    let doc_path = doc_path?;
    let dir = doc_path.parent()?;
    let joined = dir.join(src);
    Url::from_file_path(&joined).ok().map(|u| u.to_string())
}

fn generic_git_url(src: &str) -> Option<String> {
    let caps = git_re().captures(src)?;
    let domain = caps.name("domain").map(|m| m.as_str()).unwrap_or("");
    let mut repo_path = caps.name("repoPath").map(|m| m.as_str()).unwrap_or("").to_string();
    let module = caps.name("module").map(|m| m.as_str()).unwrap_or("");
    let git_ref = caps.name("ref").map(|m| m.as_str()).unwrap_or("");

    // Strip everything from the first `.git` onward.
    if let Some(idx) = repo_path.find(".git") {
        repo_path.truncate(idx);
    }
    if repo_path.is_empty() {
        return None;
    }

    let domain = if domain.is_empty() { "github.com" } else { domain };
    let git_ref = if git_ref.is_empty() { "HEAD" } else { git_ref };
    Some(format!("https://{domain}/{repo_path}/tree/{git_ref}/{module}"))
}

fn registry_module_url(src: &str) -> Option<String> {
    // Private registry: first path segment is a hostname (contains a `.`).
    let first_segment = src.split('/').next().unwrap_or("");
    if first_segment.contains('.') {
        return Some(format!("https://{src}"));
    }

    // Public registry: namespace/name/provider[/...submodules].
    let parts: Vec<&str> = src.split('/').filter(|p| !p.is_empty()).collect();
    let namespace = parts.first().copied().unwrap_or("");
    let name = parts.get(1).copied().unwrap_or("");
    let provider = parts.get(2).copied().unwrap_or("");
    let submodules = if parts.len() > 3 { &parts[3..] } else { &[][..] };

    if !submodules.is_empty() {
        let submodule = submodules.join("/").replacen("modules/", "", 1);
        return Some(format!(
            "https://registry.terraform.io/modules/{namespace}/{name}/{provider}/latest/submodules/{submodule}"
        ));
    }

    if !namespace.is_empty() && !name.is_empty() && !provider.is_empty() {
        return Some(format!(
            "https://registry.terraform.io/modules/{namespace}/{name}/{provider}"
        ));
    }

    None
}

/// Scan `text` line by line and produce document links. For each line the
/// module matcher is tried first, then the resource matcher (mirroring the
/// VSCode port).
pub fn compute_links(
    text: &str,
    provider_map: &HashMap<String, String>,
    doc_path: Option<&Path>,
) -> Vec<DocumentLink> {
    let mut links = Vec::new();

    for (line_no, line) in text.lines().enumerate() {
        let line_no = line_no as u32;

        // Module matcher first.
        if let Some(caps) = module_re().captures(line) {
            let prefix = caps.name("prefix").unwrap().as_str();
            let src = caps.name("src").unwrap().as_str();
            if let Some(url) = module_url(src, doc_path).and_then(|u| Url::parse(&u).ok()) {
                links.push(DocumentLink {
                    range: make_range(line_no, prefix, src),
                    target: Some(url.clone()),
                    tooltip: Some(url.to_string()),
                    data: None,
                });
            }
            continue;
        }

        // Then the resource/data matcher.
        if let Some(caps) = resource_re().captures(line) {
            let prefix = caps.name("prefix").unwrap().as_str();
            let kind = caps.name("kind").unwrap().as_str();
            let rtype = caps.name("rtype").unwrap().as_str();
            if let Some(url) =
                resource_url(kind, rtype, provider_map).and_then(|u| Url::parse(&u).ok())
            {
                links.push(DocumentLink {
                    range: make_range(line_no, prefix, rtype),
                    target: Some(url.clone()),
                    tooltip: Some(url.to_string()),
                    data: None,
                });
            }
        }
    }

    links
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn resource_in_provider_map() {
        let m = map(&[("cloudflare", "cloudflare/cloudflare")]);
        assert_eq!(
            resource_url("resource", "cloudflare_dns_record", &m).unwrap(),
            "https://registry.terraform.io/providers/cloudflare/cloudflare/latest/docs/resources/dns_record"
        );
    }

    #[test]
    fn resource_integrations_namespace() {
        let m = map(&[("github", "integrations/github")]);
        assert_eq!(
            resource_url("resource", "github_repository", &m).unwrap(),
            "https://registry.terraform.io/providers/integrations/github/latest/docs/resources/repository"
        );
    }

    #[test]
    fn resource_not_in_map_hashicorp_fallback() {
        let m = HashMap::new();
        assert_eq!(
            resource_url("resource", "aws_s3_bucket", &m).unwrap(),
            "https://registry.terraform.io/providers/hashicorp/aws/latest/docs/resources/s3_bucket"
        );
    }

    #[test]
    fn data_source_segment() {
        let m = HashMap::new();
        assert_eq!(
            resource_url("data", "aws_s3_bucket", &m).unwrap(),
            "https://registry.terraform.io/providers/hashicorp/aws/latest/docs/data-sources/s3_bucket"
        );
    }

    #[test]
    fn registry_module() {
        assert_eq!(
            module_url("terraform-aws-modules/vpc/aws", None).unwrap(),
            "https://registry.terraform.io/modules/terraform-aws-modules/vpc/aws"
        );
    }

    #[test]
    fn registry_submodule() {
        assert_eq!(
            module_url("terraform-aws-modules/iam/aws//modules/iam-account", None).unwrap(),
            "https://registry.terraform.io/modules/terraform-aws-modules/iam/aws/latest/submodules/iam-account"
        );
    }

    #[test]
    fn private_registry_module() {
        assert_eq!(
            module_url("app.terraform.io/example-corp/k8s-cluster/azurerm", None).unwrap(),
            "https://app.terraform.io/example-corp/k8s-cluster/azurerm"
        );
    }

    #[test]
    fn github_module() {
        assert_eq!(
            module_url("github.com/hashicorp/example", None).unwrap(),
            "https://github.com/hashicorp/example/tree/HEAD/"
        );
    }

    #[test]
    fn generic_git_module_with_ref_and_submodule() {
        assert_eq!(
            module_url("git::git@github.com:owner/repo.git//modules/name?ref=v0.0.1", None).unwrap(),
            "https://github.com/owner/repo/tree/v0.0.1/modules/name"
        );
    }

    #[test]
    fn git_ssh_module() {
        assert_eq!(
            module_url("git@github.com:hashicorp/example.git", None).unwrap(),
            "https://github.com/hashicorp/example/tree/HEAD/"
        );
    }

    #[test]
    fn http_source_skipped() {
        assert!(module_url("https://example.com/foo/bar", None).is_none());
    }

    #[test]
    fn s3_source_skipped() {
        assert!(module_url("s3::https://s3.amazonaws.com/bucket/module.zip", None).is_none());
    }

    #[test]
    fn compute_links_finds_resource_and_module() {
        let text = "resource \"aws_s3_bucket\" \"x\" {\n}\nmodule \"m\" {\n  source = \"terraform-aws-modules/vpc/aws\"\n}\n";
        let links = compute_links(text, &HashMap::new(), None);
        assert_eq!(links.len(), 2);
        // Resource link on line 0.
        assert_eq!(links[0].range.start.line, 0);
        assert_eq!(links[0].range.start.character, "resource \"".len() as u32);
        assert_eq!(links[0].range.end.character, ("resource \"".len() + "aws_s3_bucket".len()) as u32);
        assert_eq!(
            links[0].target.as_ref().unwrap().as_str(),
            "https://registry.terraform.io/providers/hashicorp/aws/latest/docs/resources/s3_bucket"
        );
        // Module link on line 3.
        assert_eq!(links[1].range.start.line, 3);
    }
}
