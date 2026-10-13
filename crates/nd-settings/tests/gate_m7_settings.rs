mod common;

mod gate_m7 {
    use std::fs;
    use std::sync::mpsc;
    use std::time::Duration;

    use nd_settings::{Event, KeyState, LayerKind, Origin, SettingsHandle, WriteError};

    use super::common::*;

    const COMMENTED: &str = "\
# Notes settings, edited by hand.
compact_rows = true   # I like it dense

# Font
editor_font_size = 15  # points, not pixels
spellcheck = false
";

    #[test]
    fn settings_round_trip_keeps_comments() {
        let dirs = Dirs::new();
        dirs.write("config.toml", COMMENTED);
        let settings = SettingsHandle::<NotesSettings>::load(dirs.options("notes")).unwrap();
        assert!(settings.get().compact_rows);

        settings.set("editor_font_size", &18.0).unwrap();
        settings.set("new_note_folder", &"pinned").unwrap();

        let text = read(&dirs.user());
        for kept in ["# Notes settings, edited by hand.", "# I like it dense", "# Font", "# points, not pixels"] {
            assert!(text.contains(kept), "lost `{kept}` in:\n{text}");
        }
        assert!(text.contains("editor_font_size = 18.0  # points, not pixels"), "{text}");
        assert!(text.contains("new_note_folder = \"pinned\""), "{text}");
        assert_eq!(settings.get().editor_font_size, 18.0);

        settings.unset("spellcheck").unwrap();
        let text = read(&dirs.user());
        assert!(!text.contains("spellcheck") && text.contains("# Font"), "{text}");
        assert!(settings.get().spellcheck, "default applies again");
    }

    #[test]
    fn store_symlinked_file_is_read_only_and_edits_go_to_the_overlay() {
        let dirs = Dirs::new();
        let managed = dirs.store.join("abc-notes-config.toml");
        fs::write(&managed, "editor_font_size = 20\n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&managed, dirs.user()).unwrap();

        let settings = SettingsHandle::<NotesSettings>::load(dirs.options("notes")).unwrap();
        assert_eq!(settings.get().editor_font_size, 20.0);
        assert!(settings.main_file_read_only());
        assert_eq!(settings.source("editor_font_size").unwrap().kind, LayerKind::Nix);
        match settings.key_state("editor_font_size") {
            KeyState::ManagedByNix { option, .. } => {
                assert_eq!(option, "programs.notes.settings.editor_font_size");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(settings.key_state("spellcheck"), KeyState::Editable);
        assert!(matches!(settings.set("editor_font_size", &12.0), Err(WriteError::ManagedByNix { .. })));
        assert_eq!(read(&managed), "editor_font_size = 20\n", "the store file is never touched");

        settings.set("spellcheck", &false).unwrap();
        assert_eq!(read(&dirs.local()), "spellcheck = false\n");
        assert!(!settings.get().spellcheck);
        assert_eq!(settings.write_target().path, dirs.local());
        assert_eq!(settings.copy_as_nix("editor_font_size").unwrap(), "programs.notes.settings.editor_font_size = 20;");
    }

    #[test]
    fn layers_merge_in_precedence_order_and_remember_their_source() {
        let dirs = Dirs::new();
        dirs.write("config.toml", "editor_font_size = 12\ncompact_rows = true\nspellcheck = false\n");
        dirs.write("config.d/10-a.toml", "editor_font_size = 13\n");
        dirs.write("config.d/20-b.toml", "editor_font_size = 14\ncompact_rows = false\n");
        dirs.write("config.local.toml", "editor_font_size = 16\n");
        let env =
            vec![("NOTES_EDITOR_FONT_SIZE".to_owned(), "18".to_owned()), ("NOTES_S3_ENDPOINT".into(), "x".into())];
        let opts = dirs.options("notes").env(env).cli(vec![("new_note_folder".into(), "pinned".into())]);
        let settings = SettingsHandle::<NotesSettings>::load(opts).unwrap();

        let value = settings.get();
        assert_eq!(value.editor_font_size, 18.0, "env beats local, drop-ins and user");
        assert!(!value.compact_rows, "later drop-in beats the user file");
        assert!(!value.spellcheck);
        assert_eq!(value.new_note_folder, super::common::NewNoteFolder::Pinned);
        assert_eq!(settings.source("editor_font_size").unwrap().kind, LayerKind::Env);
        assert_eq!(settings.source("compact_rows").unwrap().kind, LayerKind::DropIn);
        assert_eq!(settings.source("spellcheck").unwrap().kind, LayerKind::User);
        assert_eq!(settings.source("new_note_folder").unwrap().kind, LayerKind::Cli);
        assert!(matches!(settings.key_state("editor_font_size"), KeyState::Overridden { .. }));
        assert!(matches!(settings.set("editor_font_size", &20.0), Err(WriteError::Overridden { .. })));
    }

    #[test]
    fn bad_files_report_file_key_and_line() {
        let dirs = Dirs::new();
        dirs.write("config.toml", "compact_rows = true\neditor_font_size = \"big\"\n");
        let err = SettingsHandle::<NotesSettings>::load(dirs.options("notes")).err().unwrap().to_string();
        assert!(err.contains("config.toml") && err.contains("editor_font_size") && err.contains("line 2"), "{err}");

        dirs.write("config.toml", "editor_font_size = 99\n");
        let err = SettingsHandle::<NotesSettings>::load(dirs.options("notes")).err().unwrap().to_string();
        assert!(err.contains("config.toml") && err.contains("editor_font_size") && err.contains("11 to 24"), "{err}");

        dirs.write("config.toml", "editor_font_sise = 12\n");
        let err = SettingsHandle::<NotesSettings>::load(dirs.options("notes")).err().unwrap().to_string();
        assert!(err.contains("editor_font_sise"), "{err}");
    }

    #[test]
    fn a_failed_edit_writes_nothing() {
        let dirs = Dirs::new();
        dirs.write("config.toml", "# keep\ncompact_rows = true\n");
        let settings = SettingsHandle::<NotesSettings>::load(dirs.options("notes")).unwrap();
        assert!(matches!(settings.set("editor_font_size", &99.0), Err(WriteError::Invalid(_))));
        assert!(matches!(settings.set("nope", &1), Err(WriteError::UnknownKey(_))));
        assert_eq!(read(&dirs.user()), "# keep\ncompact_rows = true\n");
    }

    #[test]
    fn plugin_sections_never_break_the_strict_parse() {
        let dirs = Dirs::new();
        dirs.write(
            "config.toml",
            "compact_rows = true\n\n[plugins.discord]\napplication_id = \"123\"\nshow_state = true\n",
        );
        let settings = SettingsHandle::<NotesSettings>::load(dirs.options("notes")).unwrap();
        assert!(settings.get().compact_rows);
        let discord = settings.plugin_section("discord").unwrap();
        assert_eq!(discord["application_id"].as_str(), Some("123"));
        assert!(settings.plugin_section("other").is_none());
    }

    #[test]
    fn nested_sections_restart_keys_and_renames() {
        let dirs = Dirs::new();
        dirs.write("config.toml", "worker_threads = 4\ndark-mode = true\n[editor]\ntab_width = 2\n");
        let env = vec![("RICH_EDITOR__WRAP".to_owned(), "false".to_owned())];
        let settings = SettingsHandle::<Rich>::load(dirs.options("rich").env(env)).unwrap();
        let value = settings.get();
        assert_eq!((value.worker_threads, value.editor.tab_width, value.editor.wrap), (4, 2, false));
        assert!(value.dark_mode);

        let (tx, rx) = mpsc::channel();
        settings.on_event(move |e| {
            let _ = tx.send(e.clone());
        });
        settings.set("worker_threads", &8).unwrap();
        settings.set("editor.tab_width", &3).unwrap();
        let text = read(&dirs.user());
        assert!(text.contains("[editor]") && text.contains("tab_width = 3"), "{text}");
        let Event::Changed(first) = rx.recv().unwrap() else { panic!() };
        assert_eq!(first.restart_required, ["worker_threads"]);
        assert_eq!(first.origin, Origin::Own);
        let Event::Changed(second) = rx.recv().unwrap() else { panic!() };
        assert_eq!(second.changed, ["editor.tab_width"]);
        assert!(second.restart_required.is_empty());
        assert!(matches!(settings.set("editor.tab_width", &9), Err(WriteError::Invalid(_))));
    }

    #[test]
    fn watcher_reloads_external_edits_and_ignores_its_own_writes() {
        let dirs = Dirs::new();
        dirs.write("config.toml", "compact_rows = false\n");
        let settings = SettingsHandle::<NotesSettings>::load(dirs.options("notes")).unwrap();
        let (tx, rx) = mpsc::channel();
        settings.on_event(move |e| {
            let _ = tx.send(e.clone());
        });
        let _guard = settings.watch().unwrap();

        settings.set("spellcheck", &false).unwrap();
        let Event::Changed(own) = rx.recv_timeout(Duration::from_secs(2)).unwrap() else { panic!() };
        assert_eq!(own.origin, Origin::Own);
        assert_eq!(own.changed, ["spellcheck"]);
        assert!(rx.recv_timeout(Duration::from_millis(700)).is_err(), "own write must not reload again");

        fs::write(dirs.user(), "compact_rows = true\nspellcheck = false\n").unwrap();
        let Event::Changed(external) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else { panic!() };
        assert_eq!(external.origin, Origin::External);
        assert_eq!(external.changed, ["compact_rows"]);
        assert!(settings.get().compact_rows);

        fs::write(dirs.user(), "compact_rows = \"yes\"\n").unwrap();
        let Event::Rejected(why) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else { panic!() };
        assert!(why.contains("compact_rows"), "{why}");
        assert!(settings.get().compact_rows, "last good settings stay");

        fs::write(dirs.user(), "compact_rows = false\nspellcheck = false\n").unwrap();
        let Event::Changed(fixed) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else { panic!() };
        assert_eq!(fixed.changed, ["compact_rows"]);
    }

    #[test]
    fn watcher_sees_a_symlink_swap_into_the_store() {
        let dirs = Dirs::new();
        dirs.write("config.toml", "compact_rows = false\n");
        let settings = SettingsHandle::<NotesSettings>::load(dirs.options("notes")).unwrap();
        assert!(!settings.main_file_read_only());
        let (tx, rx) = mpsc::channel();
        settings.on_event(move |e| {
            let _ = tx.send(e.clone());
        });
        let _guard = settings.watch().unwrap();

        // What a home-manager switch does: replace the file with a link to the store.
        let managed = dirs.store.join("gen2-config.toml");
        fs::write(&managed, "compact_rows = true\n").unwrap();
        let link = dirs.config.join("swap");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&managed, &link).unwrap();
        fs::rename(&link, dirs.user()).unwrap();

        let Event::Changed(change) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else { panic!() };
        assert_eq!(change.changed, ["compact_rows"]);
        assert!(settings.main_file_read_only());
        assert!(matches!(settings.key_state("compact_rows"), KeyState::ManagedByNix { .. }));
    }
}
