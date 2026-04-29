// Returns the list of installed apps with UIFileSharingEnabled=true.

use idevice::{
    IdeviceService,
    installation_proxy::InstallationProxyClient,
    provider::IdeviceProvider,
};

use super::AppInfo;

pub async fn list_apps_info(
    provider: &dyn IdeviceProvider,
) -> Result<Vec<AppInfo>, Box<dyn std::error::Error>> {
    let mut instproxy = InstallationProxyClient::connect(provider).await?;
    let apps = instproxy.get_apps(Some("User"), None).await?;

    let mut result = Vec::new();
    for (bundle_id, info) in &apps {
        let file_sharing = info
            .as_dictionary()
            .and_then(|d| d.get("UIFileSharingEnabled"))
            .and_then(|v| v.as_boolean())
            .unwrap_or(false);

        if file_sharing {
            let display_name = info
                .as_dictionary()
                .and_then(|d| d.get("CFBundleDisplayName").or_else(|| d.get("CFBundleName")))
                .and_then(|v| v.as_string())
                .unwrap_or(bundle_id.as_str())
                .to_owned();

            result.push(AppInfo {
                bundle_id: bundle_id.clone(),
                display_name,
                icon_png: None,  // populated later by fetch_app_icons()
            });
        }
    }

    Ok(result)
}
