use serde::Deserialize;

pub const PLAN_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallPlan {
    #[serde(default)]
    pub schema_version: Option<u32>,
    pub items: Vec<InstallPlanItem>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallPlanItem {
    pub id: String,
    pub method: String,
    pub version: Option<String>,
    pub url: Option<String>,
    pub sha256: Option<String>,
    pub archive_binary: Option<String>,
    pub binary_name: Option<String>,
    pub destination: Option<String>,
    pub package: Option<String>,
    pub manager: Option<String>,
    pub python: Option<String>,
}
