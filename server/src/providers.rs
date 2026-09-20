//! Parsing of Terraform `required_providers` blocks into an
//! `alias -> "namespace/providerName"` map. Ported from the VSCode extension's
//! `src/terraform/provider.ts`.

use std::collections::HashMap;
use std::path::Path;

use regex::Regex;

/// Parse all `required_providers` blocks in `content` into a map of
/// `alias -> source` (source is `namespace/providerName`).
///
/// This is a faithful port of the string-aware, brace-balancing parser in
/// `provider.ts`. It operates on bytes because every character it inspects
/// (`{`, `}`, `"`, `'`, `#`, `/`, `\`, `\n`) is ASCII, and UTF-8 multi-byte
/// sequences never contain those byte values.
pub fn parse_providers(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let bytes = content.as_bytes();
    let keyword = "required_providers";

    // Regex matching a single provider entry inside an isolated block.
    let provider_re =
        Regex::new(r#"(?P<alias>[\w-]+)\s*=\s*\{[\s\S]*?source\s*=\s*"(?P<source>[^"]+)""#).unwrap();

    let mut pos = 0usize;
    loop {
        // Find the next `required_providers` keyword.
        let found = match content[pos..].find(keyword) {
            Some(i) => pos + i,
            None => break,
        };
        pos = found + keyword.len();

        // Find the opening brace.
        let open_brace = match content[pos..].find('{') {
            Some(i) => pos + i,
            None => break,
        };

        // Only whitespace (and optionally a single `=`) may sit between the
        // keyword and the brace. Supports `required_providers {` and
        // `required_providers = {`.
        let gap = &content[pos..open_brace];
        if !gap.replace('=', "").trim().is_empty() {
            continue;
        }

        // Balance braces to isolate the block, being string- and
        // comment-aware.
        let mut balance: i32 = 1;
        let mut i = open_brace + 1;
        let mut in_string = false;
        let mut string_char = 0u8;

        while i < bytes.len() && balance > 0 {
            let c = bytes[i];
            if in_string {
                if c == string_char && bytes[i - 1] != b'\\' {
                    in_string = false;
                }
            } else if c == b'"' || c == b'\'' {
                in_string = true;
                string_char = c;
            } else if c == b'{' {
                balance += 1;
            } else if c == b'}' {
                balance -= 1;
            } else if c == b'#' {
                // `#` line comment: skip to end of line.
                if let Some(nl) = content[i..].find('\n') {
                    i += nl;
                    continue;
                }
            } else if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                // `//` line comment: skip to end of line.
                if let Some(nl) = content[i..].find('\n') {
                    i += nl;
                    continue;
                }
            }
            i += 1;
        }

        if balance == 0 {
            let block = &content[open_brace + 1..i - 1];
            for cap in provider_re.captures_iter(block) {
                let alias = cap.name("alias").map(|m| m.as_str()).unwrap_or("");
                let source = cap.name("source").map(|m| m.as_str()).unwrap_or("");
                if !alias.is_empty() && !source.is_empty() {
                    map.insert(alias.to_string(), source.to_string());
                }
            }
            pos = i;
        } else {
            // Malformed (unclosed) block: stop to avoid looping.
            break;
        }
    }

    map
}

/// Build the provider map for a document. Sibling `versions.tf`, `main.tf`, and
/// `providers.tf` files are read from disk (missing files ignored); the current
/// document's live text is processed last so it overrides on-disk definitions.
pub fn get_provider_map(doc_path: Option<&Path>, doc_text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();

    if let Some(path) = doc_path {
        if let Some(dir) = path.parent() {
            let current_name = path.file_name().and_then(|s| s.to_str());
            for file in ["versions.tf", "main.tf", "providers.tf"] {
                // Skip the current file here; it is processed last from the
                // live buffer text.
                if Some(file) == current_name {
                    continue;
                }
                let file_path = dir.join(file);
                if let Ok(content) = std::fs::read_to_string(&file_path) {
                    for (alias, source) in parse_providers(&content) {
                        map.insert(alias, source);
                    }
                }
            }
        }
    }

    // Current document last so it overrides.
    for (alias, source) in parse_providers(doc_text) {
        map.insert(alias, source);
    }

    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_braced_and_equals_forms() {
        // First block uses `required_providers {`, second uses `= {`.
        let content = r#"
terraform {
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 6.0"
    }
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "~> 5.0"
    }
  }
}

terraform {
  required_providers = {
    github = {
      source = "integrations/github"
    }
  }
}
"#;
        let map = parse_providers(content);
        assert_eq!(map.get("aws").map(String::as_str), Some("hashicorp/aws"));
        assert_eq!(
            map.get("cloudflare").map(String::as_str),
            Some("cloudflare/cloudflare")
        );
        assert_eq!(map.get("github").map(String::as_str), Some("integrations/github"));
    }

    #[test]
    fn parses_with_comments_inside_block() {
        let content = r#"
terraform {
  required_providers {
    # this is a hash comment mentioning a stray brace { that must be ignored
    aws = {
      source  = "hashicorp/aws" // inline slash comment with } brace
    }
    // slash comment mentioning another } brace
    datadog = {
      source = "datadog/datadog"
    }
  }
}
"#;
        let map = parse_providers(content);
        assert_eq!(map.get("aws").map(String::as_str), Some("hashicorp/aws"));
        assert_eq!(map.get("datadog").map(String::as_str), Some("datadog/datadog"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn ignores_braces_inside_strings() {
        let content = r#"
terraform {
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      # a string with an unbalanced brace should not break parsing:
      note    = "a } brace in a string"
    }
  }
}
"#;
        let map = parse_providers(content);
        assert_eq!(map.get("aws").map(String::as_str), Some("hashicorp/aws"));
    }

    #[test]
    fn no_required_providers_yields_empty() {
        let map = parse_providers("resource \"aws_s3_bucket\" \"x\" {}\n");
        assert!(map.is_empty());
    }
}
