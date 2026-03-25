use omne_host_info_primitives::detect_host_platform;
use omne_system_package_primitives::{
    SystemPackageInstallRecipe, default_system_package_install_recipes_for_os,
};

pub(crate) fn default_current_host_system_package_install_recipes(
    package: &str,
) -> Vec<SystemPackageInstallRecipe> {
    let Some(platform) = detect_host_platform() else {
        return Vec::new();
    };
    default_system_package_install_recipes_for_os(platform.operating_system().as_str(), package)
}
