use serde_json::json;

use switchy_lib::{
    get_kimi_config_path, get_kimi_credentials_path, read_kimi_live, write_kimi_live_atomic,
    AppType, MultiAppConfig, Provider, ProviderService,
};

#[path = "support.rs"]
mod support;
use support::{create_test_state_with_config, ensure_test_home, reset_test_fs, test_mutex};

const OFFICIAL_CONFIG: &str = r#"default_model = "kimi-code/k3"

[providers."managed:kimi-code"]
type = "kimi"
api_key = ""
base_url = "https://api.kimi.com/coding/v1"

[models."kimi-code/k3"]
provider = "managed:kimi-code"
model = "k3"
"#;

const THIRD_PARTY_CONFIG: &str = r#"default_model = "relay/gpt-4o"

[providers.relay]
type = "openai"
api_key = "sk-relay"
base_url = "https://relay.example/v1"

[models."relay/gpt-4o"]
provider = "relay"
model = "gpt-4o"
"#;

fn kimi_provider(id: &str, config: &str, credentials: serde_json::Value) -> Provider {
    Provider::with_id(
        id.to_string(),
        id.to_string(),
        json!({ "config": config, "credentials": credentials }),
        None,
    )
}

#[test]
fn switching_kimi_writes_config_and_credentials_and_backfills_the_outgoing_login() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    // Live state: the official provider is current and Kimi refreshed its login
    // since Switchy last saw it.
    let refreshed = json!({
        "access_token": "AAA",
        "refresh_token": "RRR-refreshed",
        "expires_at": 1_789_183_300,
        "scope": "kimi-code",
        "token_type": "Bearer"
    });
    write_kimi_live_atomic(Some(&refreshed), OFFICIAL_CONFIG).expect("seed live kimi");

    let mut config = MultiAppConfig::default();
    {
        let manager = config
            .get_manager_mut(&AppType::Kimi)
            .expect("kimi manager exists by default");
        manager.current = "official".to_string();
        manager.providers.insert(
            "official".to_string(),
            kimi_provider(
                "official",
                OFFICIAL_CONFIG,
                json!({ "access_token": "old", "refresh_token": "RRR-old" }),
            ),
        );
        manager.providers.insert(
            "relay".to_string(),
            kimi_provider("relay", THIRD_PARTY_CONFIG, serde_json::Value::Null),
        );
    }
    let state = create_test_state_with_config(&config).expect("create test state");

    ProviderService::switch(&state, AppType::Kimi, "relay").expect("switch to relay");

    // Live files now describe the relay provider; the managed login is gone.
    let live_config = std::fs::read_to_string(get_kimi_config_path()).expect("read config.toml");
    assert!(live_config.contains("default_model = \"relay/gpt-4o\""));
    assert!(live_config.contains("api_key = \"sk-relay\""));
    assert!(
        !get_kimi_credentials_path().exists(),
        "an API-key provider carries no managed login"
    );

    // The outgoing official provider picked up the refreshed login.
    let providers = state
        .db
        .get_all_providers("kimi")
        .expect("read kimi providers");
    let official = providers.get("official").expect("official still exists");
    assert_eq!(
        official.settings_config["credentials"]["refresh_token"], "RRR-refreshed",
        "switch-away backfill stores the live login"
    );

    // Switching back restores that login on disk.
    ProviderService::switch(&state, AppType::Kimi, "official").expect("switch back");
    let live = read_kimi_live().expect("read live");
    assert_eq!(live["credentials"]["refresh_token"], "RRR-refreshed");
    assert!(live["config"]
        .as_str()
        .unwrap()
        .contains("managed:kimi-code"));

    let current = state
        .db
        .get_current_provider("kimi")
        .expect("read current provider");
    assert_eq!(current.as_deref(), Some("official"));
}

#[test]
fn kimi_provider_without_config_is_rejected() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    let mut config = MultiAppConfig::default();
    {
        let manager = config
            .get_manager_mut(&AppType::Kimi)
            .expect("kimi manager");
        manager.providers.insert(
            "broken".to_string(),
            Provider::with_id(
                "broken".to_string(),
                "Broken".to_string(),
                json!({ "credentials": null }),
                None,
            ),
        );
    }
    let state = create_test_state_with_config(&config).expect("create test state");

    ProviderService::switch(&state, AppType::Kimi, "broken")
        .expect_err("a Kimi provider without config.toml text cannot be switched to");
    assert!(!get_kimi_config_path().exists());
}
