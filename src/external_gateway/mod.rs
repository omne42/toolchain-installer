mod asset_routing;

#[cfg(test)]
pub(crate) use asset_routing::make_gateway_asset_candidate;
pub(crate) use asset_routing::{
    gateway_candidate_for_git_release_asset, infer_gateway_candidate_for_git_release,
};
