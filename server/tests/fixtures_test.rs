//! Integration tests exercising the fixture `.tf` files copied from the
//! upstream VSCode extension.

// Pull the library modules in as source (the crate is a binary-only crate).
#[path = "../src/links.rs"]
mod links;
#[path = "../src/providers.rs"]
mod providers;

const COMMUNITY: &str = include_str!("fixtures/community_providers.tf");
const MAIN: &str = include_str!("fixtures/main.tf");

#[test]
fn community_providers_map_and_links() {
    let map = providers::parse_providers(COMMUNITY);
    assert_eq!(map.get("cloudflare").map(String::as_str), Some("cloudflare/cloudflare"));
    assert_eq!(map.get("digitalocean").map(String::as_str), Some("digitalocean/digitalocean"));
    assert_eq!(map.get("datadog").map(String::as_str), Some("datadog/datadog"));
    assert_eq!(map.get("github").map(String::as_str), Some("integrations/github"));
    assert_eq!(map.get("proxmox").map(String::as_str), Some("bpg/proxmox"));

    let links = links::compute_links(COMMUNITY, &map, None);
    let targets: Vec<String> = links
        .iter()
        .filter_map(|l| l.target.as_ref().map(|u| u.to_string()))
        .collect();

    assert!(targets.contains(
        &"https://registry.terraform.io/providers/cloudflare/cloudflare/latest/docs/resources/dns_record"
            .to_string()
    ));
    assert!(targets.contains(
        &"https://registry.terraform.io/providers/integrations/github/latest/docs/resources/repository"
            .to_string()
    ));
    assert!(targets.contains(
        &"https://registry.terraform.io/providers/bpg/proxmox/latest/docs/resources/virtual_environment_download_file"
            .to_string()
    ));
}

#[test]
fn main_tf_links() {
    let map = providers::parse_providers(MAIN);
    let links = links::compute_links(MAIN, &map, None);
    let targets: Vec<String> = links
        .iter()
        .filter_map(|l| l.target.as_ref().map(|u| u.to_string()))
        .collect();

    // aws is in the map (hashicorp/aws) → resources + data-sources.
    assert!(targets.contains(
        &"https://registry.terraform.io/providers/hashicorp/aws/latest/docs/resources/s3_bucket"
            .to_string()
    ));
    assert!(targets.contains(
        &"https://registry.terraform.io/providers/hashicorp/aws/latest/docs/data-sources/s3_bucket"
            .to_string()
    ));
    // registry modules
    assert!(targets.contains(
        &"https://registry.terraform.io/modules/terraform-aws-modules/vpc/aws".to_string()
    ));
    assert!(targets.contains(
        &"https://registry.terraform.io/modules/terraform-aws-modules/iam/aws/latest/submodules/iam-account"
            .to_string()
    ));
    // private registry
    assert!(targets.contains(&"https://app.terraform.io/example-corp/k8s-cluster/azurerm".to_string()));
    // github + generic git
    assert!(targets.contains(&"https://github.com/hashicorp/example/tree/HEAD/".to_string()));
    assert!(targets.contains(&"https://github.com/owner/repo/tree/v0.0.1/modules/name".to_string()));
}
