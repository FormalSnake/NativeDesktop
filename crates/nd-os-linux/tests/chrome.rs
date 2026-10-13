//! Tier 1: the chrome policy (ARCHITECTURE.md 5.4) for every decoration answer a compositor gives.

mod gate_m2 {
    use nd_os_linux::chrome::{
        ButtonLayout, Capabilities, ChromeState, Control, ControlMetrics, Decorations, describe, plan,
    };

    fn state(decorations: Decorations, capabilities: Capabilities, layout: &str) -> ChromeState {
        ChromeState { decorations, capabilities, layout: ButtonLayout::parse(layout), fullscreen: false }
    }

    /// What Hyprland sends: server-side whatever the client asked, and `wm_capabilities` of
    /// maximize and fullscreen (`src/protocols/XDGShell.cpp` and `XDGDecoration.cpp`, v0.56.2).
    fn hyprland() -> ChromeState {
        let caps = Capabilities { maximize: true, minimize: false, fullscreen: true, window_menu: false };
        state(Decorations::Server, caps, "appmenu:minimize,maximize,close")
    }

    #[test]
    fn hyprland_gets_no_controls_and_no_insets() {
        let p = plan(&hyprland(), ControlMetrics::default());
        assert!(p.is_empty(), "{p:?}");
        assert_eq!((p.leading_inset, p.trailing_inset), (0., 0.));
        assert_eq!(
            describe(&hyprland(), &p),
            "decorations=server controls=none caps=maximize,fullscreen fullscreen=false insets=0,0"
        );
    }

    #[test]
    fn server_side_draws_nothing_even_with_every_capability() {
        let s = state(Decorations::Server, Capabilities::default(), "close,minimize,maximize:");
        assert!(plan(&s, ControlMetrics::default()).is_empty());
    }

    #[test]
    fn client_side_draws_what_wm_capabilities_lists() {
        let none = Capabilities { maximize: false, minimize: false, fullscreen: false, window_menu: false };
        let p = plan(&state(Decorations::Client, none, "appmenu:minimize,maximize,close"), ControlMetrics::default());
        assert_eq!(p.trailing, vec![Control::Close], "close is not a capability, it is always there");

        let maximize_only = Capabilities { maximize: true, ..none };
        let p = plan(&state(Decorations::Client, maximize_only, ":minimize,maximize,close"), ControlMetrics::default());
        assert_eq!(p.trailing, vec![Control::Maximize, Control::Close]);
    }

    #[test]
    fn without_wm_capabilities_everything_is_supported() {
        // xdg-shell before v5 never sends the event; the protocol says the client assumes all.
        let p = plan(
            &state(Decorations::Client, Capabilities::default(), "appmenu:minimize,maximize,close"),
            ControlMetrics::default(),
        );
        assert_eq!(p.trailing, vec![Control::Minimize, Control::Maximize, Control::Close]);
    }

    #[test]
    fn button_layout_picks_the_side_and_insets_follow() {
        let m = ControlMetrics::default();
        let p = plan(&state(Decorations::Client, Capabilities::default(), "close,minimize:"), m);
        assert_eq!(p.leading, vec![Control::Close, Control::Minimize]);
        assert!(p.trailing.is_empty());
        assert_eq!(p.leading_inset, m.padding * 2. + m.size * 2. + m.spacing);
        assert_eq!(p.trailing_inset, 0.);
    }

    #[test]
    fn button_layout_skips_unknown_names_and_repeats() {
        let l = ButtonLayout::parse("appmenu,icon:spacer,close,close,maximize");
        assert_eq!(l, ButtonLayout { leading: vec![], trailing: vec![Control::Close, Control::Maximize] });
        assert_eq!(ButtonLayout::parse("minimize,close"), ButtonLayout {
            leading: vec![],
            trailing: vec![Control::Minimize, Control::Close]
        });
    }

    #[test]
    fn full_screen_hides_client_side_controls() {
        let mut s = state(Decorations::Client, Capabilities::default(), "appmenu:close");
        s.fullscreen = true;
        assert!(plan(&s, ControlMetrics::default()).is_empty());
    }
}
