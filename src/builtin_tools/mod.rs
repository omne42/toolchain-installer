pub(crate) mod bootstrap_installation;
pub(crate) mod bootstrap_tool_health;
pub(crate) mod builtin_tool_selection;
pub(crate) mod public_release_asset_installation;

#[cfg(test)]
pub(crate) use bootstrap_installation::builtin_tool_destination;
#[cfg(test)]
pub(crate) use bootstrap_tool_health::{
    ManagedBootstrapState, assess_managed_bootstrap_state, host_command_is_healthy,
};
#[cfg(test)]
pub(crate) use builtin_tool_selection::normalize_requested_tools;
#[cfg(test)]
pub(crate) use public_release_asset_installation::{
    gh_release_asset_suffix_for_target, install_gh_from_public_release,
    install_git_from_public_release, replace_mingit_installation,
    select_mingit_release_asset_for_target,
};
