mod download_candidates;
mod download_source_kind_mapping;
mod download_transfer;
mod gateway_asset_routing;
mod github_release_metadata;

#[cfg(test)]
pub(crate) use download_candidates::make_download_candidates;
pub(crate) use download_candidates::{DownloadCandidate, build_download_candidates};
pub(crate) use download_source_kind_mapping::result_source_kind_for_download_candidate;
pub(crate) use download_transfer::{DownloadOptions, download_candidate_to_writer_with_options};
pub(crate) use gateway_asset_routing::{
    infer_gateway_candidate_for_git_release, make_gateway_asset_candidate,
};
pub(crate) use github_release_metadata::{GithubReleaseAsset, fetch_latest_github_release};
