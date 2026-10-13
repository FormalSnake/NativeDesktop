mod gate_m7_plugin {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use nd_plugin::{Plugin, PluginContext, PluginError, PluginHost, Services};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct AppConfig {
        title: String,
    }

    #[derive(Default, Deserialize, Serialize)]
    #[serde(deny_unknown_fields, default)]
    struct DemoConfig {
        enabled: bool,
        label: String,
    }

    #[derive(Clone, Default)]
    struct Counters {
        inits: Arc<AtomicUsize>,
        shutdowns: Arc<AtomicUsize>,
        ticks: Arc<AtomicUsize>,
    }

    struct Demo {
        name: &'static str,
        counters: Counters,
    }

    impl Plugin for Demo {
        type Config = DemoConfig;

        fn name(&self) -> &'static str {
            self.name
        }

        fn init(&mut self, cx: &PluginContext, config: DemoConfig) -> Result<(), PluginError> {
            self.counters.inits.fetch_add(1, Ordering::SeqCst);
            if config.enabled {
                let ticks = Arc::clone(&self.counters.ticks);
                cx.spawn(async move {
                    loop {
                        ticks.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                });
            }
            Ok(())
        }

        fn shutdown(&mut self) {
            self.counters.shutdowns.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn host(counters: &Counters) -> PluginHost {
        let mut host = PluginHost::new("gate-app", tokio::runtime::Handle::current(), Services::new());
        host.register(Demo { name: "demo", counters: counters.clone() });
        host
    }

    fn table(src: &str) -> toml::Table {
        src.parse().unwrap()
    }

    #[tokio::test]
    async fn plugin_sections_never_reach_the_apps_strict_parse() {
        let src = r#"
            title = "Notes"
            [plugins.demo]
            enabled = false
            label = "x"
            [plugins.not_installed]
            anything = 1
        "#;
        assert!(table(src).try_into::<AppConfig>().is_err(), "premise: the raw file trips deny_unknown_fields");

        let counters = Counters::default();
        let mut root = table(src);
        let report = host(&counters).load(&mut root);
        let app: AppConfig = root.try_into().unwrap();
        assert_eq!(app.title, "Notes");
        assert_eq!(report.started, ["demo"]);
        assert_eq!(report.unclaimed, ["not_installed"]);
    }

    #[tokio::test]
    async fn a_bad_section_disables_only_its_plugin() {
        let counters = Counters::default();
        let mut host = host(&counters);
        let mut root = table("title = \"Notes\"\n[plugins.demo]\nenabeld = true\n");
        let report = host.load(&mut root);
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].error.to_string().contains("enabeld"));
        assert!(root.try_into::<AppConfig>().is_ok());
        assert_eq!(counters.inits.load(Ordering::SeqCst), 0);

        let mut fixed = table("title = \"Notes\"\n[plugins.demo]\nenabled = true\n");
        assert_eq!(host.reload(&mut fixed).started, ["demo"]);
        assert_eq!(counters.inits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn plugins_key_of_the_wrong_type_is_reported_not_fatal() {
        let counters = Counters::default();
        let mut root = table("title = \"Notes\"\nplugins = 3\n");
        let report = host(&counters).load(&mut root);
        assert!(report.malformed_plugins_key.is_some());
        assert!(root.try_into::<AppConfig>().is_ok());
    }

    #[tokio::test]
    async fn reload_keeps_the_old_config_when_the_new_one_is_bad_and_shutdown_stops_tasks() {
        let counters = Counters::default();
        let mut host = host(&counters);
        host.load(&mut table("[plugins.demo]\nenabled = true\n"));
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(counters.ticks.load(Ordering::SeqCst) > 1);

        let report = host.reload(&mut table("[plugins.demo]\nlabel = 5\n"));
        assert_eq!(report.failed.len(), 1);
        assert_eq!(counters.inits.load(Ordering::SeqCst), 1, "bad reload must not restart the plugin");

        host.shutdown();
        assert_eq!(counters.shutdowns.load(Ordering::SeqCst), 1);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let settled = counters.ticks.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(counters.ticks.load(Ordering::SeqCst), settled);
    }

    #[tokio::test]
    async fn defaults_and_services() {
        let counters = Counters::default();
        let host = host(&counters);
        let defaults = host.default_sections();
        assert_eq!(defaults["demo"]["enabled"].as_bool(), Some(false));

        let services = Services::new();
        services.insert(41u32);
        services.insert(String::from("shared"));
        assert_eq!(services.get::<u32>().as_deref(), Some(&41));
        services.insert(42u32);
        assert_eq!(services.get::<u32>().as_deref(), Some(&42));
        assert!(services.get::<i8>().is_none());
    }
}
