//! 将已持久化配置应用到正在运行的信令节点。

use std::collections::HashMap;

use crate::AppState;

const HOT_RELOAD_KEYS: [&str; 4] = [
    "recording_enabled",
    "recording_min_free_bytes",
    "recording_max_file_bytes",
    "recording_max_duration_secs",
];

#[derive(Debug, Default)]
pub struct ConfigReloadOutcome {
    pub applied: Vec<String>,
    pub error: Option<String>,
}

/// 判断配置项是否支持在新录音会话开始前热更新。
pub fn is_hot_reload_key(key: &str) -> bool {
    HOT_RELOAD_KEYS.contains(&key)
}

/// 通知当前信令节点从数据库重新加载安全的录音配置子集。
pub async fn apply_recording_config(
    state: &AppState,
    payload: &HashMap<String, String>,
) -> ConfigReloadOutcome {
    let mut requested: Vec<_> = payload
        .keys()
        .filter(|key| is_hot_reload_key(key))
        .cloned()
        .collect();
    requested.sort();
    if requested.is_empty() {
        return ConfigReloadOutcome::default();
    }

    let url = format!("{}/manage/config/recording", state.sip_manage_base);
    match state
        .internal_client
        .put(url)
        .header("X-VOS-Token", &state.internal_secret)
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => ConfigReloadOutcome {
            applied: requested,
            error: None,
        },
        Ok(response) => {
            let status = response.status();
            tracing::warn!(%status, "信令节点拒绝录音配置热更新");
            ConfigReloadOutcome {
                applied: Vec::new(),
                error: Some(format!("信令节点热更新失败（状态码 {status}）")),
            }
        }
        Err(error) => {
            tracing::warn!(%error, "请求信令节点热更新录音配置失败");
            ConfigReloadOutcome {
                applied: Vec::new(),
                error: Some("信令节点暂不可用，配置将在节点重启后生效".to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_hot_reload_key;

    #[test]
    fn classifies_only_safe_recording_fields_as_hot_reloadable() {
        assert!(is_hot_reload_key("recording_enabled"));
        assert!(is_hot_reload_key("recording_max_duration_secs"));
        assert!(!is_hot_reload_key("recording_dir"));
        assert!(!is_hot_reload_key("recording_workers"));
    }
}
