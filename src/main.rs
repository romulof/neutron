mod config;
mod launch;
mod ui;

use adw::prelude::*;
use gtk4::gio;
use libadwaita as adw;
use std::env;

const APP_ID: &str = "com.github.romulof.neutron";

fn setup_css() {
    let provider = gtk4::CssProvider::new();
    provider.load_from_data(
        "
        .monospace {
            font-family: monospace;
            font-size: 13px;
        }
        entry.vram-entry text {
            padding-right: 32px;
        }
        entry.timeout-entry text {
            padding-right: 24px;
        }
        entry.env-key-entry text {
            padding-right: 0;
            transition: padding-right 150ms ease-in-out;
        }
        entry.env-key-entry.show-check text {
            padding-right: 28px;
        }
        entry.env-key-entry.warning image {
            margin-right: 0;
            transition: margin-right 150ms ease-in-out;
        }
        entry.env-key-entry.warning.show-check image {
            margin-right: 28px;
        }
        entry.env-key-entry.warning.show-check text {
            padding-right: 0;
        }
        checkbutton.env-key-check {
            padding: 0;
            min-width: 0;
            min-height: 0;
            opacity: 0;
            transition: opacity 150ms ease-in-out;
        }
        checkbutton.env-key-check.show-check {
            opacity: 1;
        }
        checkbutton.env-key-check > check {
            margin: 0;
        }
        .game-header-banner {
            border-radius: 8px;
        }
        .strikethrough {
            text-decoration-line: line-through;
            opacity: 0.55;
        }
        entry.disabled-entry {
            opacity: 0.55;
        }
        .dirty-dot {
            color: @accent_color;
            font-size: 14px;
            margin-left: 4px;
        }
        entry.modified-entry {
            border-color: @accent_color;
        }
        row.modified-row {
            border-left: 4px solid @accent_color;
        }
        .game-icon {
            border-radius: 6px;
        }
        .table-header {
            font-weight: bold;
            font-size: 12px;
        }
        .env-card {
            border-radius: 8px;
        }
        button.add-game-subtle {
            opacity: 0.65;
            transition: opacity 150ms ease-in-out;
        }
        button.add-game-subtle:hover {
            opacity: 1;
        }
        entry.env-val-entry text {
            padding-right: 0;
            transition: padding-right 150ms ease-in-out;
        }
        entry.env-val-entry.show-trash text {
            padding-right: 28px;
        }
        button.env-val-delete {
            padding: 0;
            min-width: 24px;
            min-height: 24px;
            opacity: 0;
            transition: opacity 150ms ease-in-out;
        }
        button.env-val-delete.show-trash {
            opacity: 1;
        }
        .dlss-note {
            font-size: 11px;
            opacity: 0.7;
        }
        entry.warning {
            border-color: @warning_color;
        }
        entry.warning image {
            color: @warning_color;
        }
        popover.warning-popover {
            background-color: transparent;
        }
        popover.warning-popover > arrow,
        popover.warning-popover > contents {
            background-color: RGB(0 0 6/85%);
            color: white;
        }
        popover.warning-popover > contents {
            padding: 6px 10px;
            border-radius: 9px;
            border: 1px solid RGB(255 255 255/12%);
            box-shadow: 0 2px 10px rgba(0, 0, 0, 0.5);
            font-size: 12px;
            min-height: 0;
        }
        popover.warning-popover,
        popover.warning-popover label,
        .warning-popover-label {
            font-family: sans-serif;
            font-size: 12px;
            color: white;
        }
        box.toolbar.bottom-action-bar {
            padding: 12px 24px;
            border-spacing: 10px;
        }
        row.expander.compact-expander row.header > box.header {
            min-height: 30px;
        }
        row.expander.compact-expander row.header > box.header > box.title {
            margin-top: 2px;
            margin-bottom: 2px;
        }
        ",
    );
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(&display, &provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}

fn setup_icons() {
    if let Some(display) = gtk4::gdk::Display::default() {
        let theme = gtk4::IconTheme::for_display(&display);

        // Look in package icons directory during development
        theme.add_search_path(concat!(env!("CARGO_MANIFEST_DIR"), "/icons"));

        // Look relative to current executable if run after deployment
        if let Ok(exe_path) = env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                theme.add_search_path(exe_dir.join("icons"));
                theme.add_search_path(exe_dir.join("../icons"));
                theme.add_search_path(exe_dir.join("../share/icons"));
                theme.add_search_path(exe_dir.join("../share/icons/hicolor/scalable/apps"));
            }
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let mode = launch::detect_mode(&args);

    match mode {
        launch::LaunchMode::CompatVerb { verb, args } => {
            // Steam query/setup verb (getcompatpath, runinprefix, destroyprefix, etc.)
            // Never init GTK. Exec Proton immediately and silently.
            launch::exec_compat_verb(&verb, &args);
        }
        launch::LaunchMode::SilentGame { verb, app_id, args } => {
            // Install script evaluator (iscriptevaluator.exe) or verb "run"
            // Never init GTK. Exec Proton with game environment immediately and silently.
            launch::exec_compat_game(&verb, &app_id, &args);
        }
        launch::LaunchMode::CompatGame { verb, app_id, args } => {
            let timeout = config::get_launcher_timeout();
            if timeout.is_zero() {
                launch::exec_compat_game(&verb, &app_id, &args);
            }

            launch::enable_software_renderer_for_ui();
            let app = adw::Application::builder()
                .application_id(APP_ID)
                .flags(gio::ApplicationFlags::NON_UNIQUE)
                .build();

            let verb_c = verb.clone();
            let app_id_c = app_id.clone();
            let args_c = args.clone();

            app.connect_activate(move |app| {
                setup_css();
                setup_icons();

                let verb = verb_c.clone();
                let app_id = app_id_c.clone();
                let args = args_c.clone();

                let app_id_for_launch = app_id.clone();
                let launcher = ui::launcher::LauncherWindow::new(app, &app_id, None, move |_| {
                    launch::exec_compat_game(&verb, &app_id_for_launch, &args);
                });
                launcher.present();
            });

            app.run_with_args::<&str>(&[]);
        }
        launch::LaunchMode::CommandWrap { app_id, command } => {
            let timeout = config::get_launcher_timeout();
            if timeout.is_zero() {
                launch::exec_command_wrap(&app_id, &command);
            }

            launch::enable_software_renderer_for_ui();
            let app = adw::Application::builder()
                .application_id(APP_ID)
                .flags(gio::ApplicationFlags::NON_UNIQUE)
                .build();

            let app_id_c = app_id.clone();
            let command_c = command.clone();

            app.connect_activate(move |app| {
                setup_css();
                setup_icons();

                let app_id = app_id_c.clone();
                let command = command_c.clone();

                let app_id_for_launch = app_id.clone();
                let launcher = ui::launcher::LauncherWindow::new(app, &app_id, None, move |_| {
                    launch::exec_command_wrap(&app_id_for_launch, &command);
                });
                launcher.present();
            });

            app.run_with_args::<&str>(&[]);
        }
        launch::LaunchMode::Preview { app_id } => {
            launch::enable_software_renderer_for_ui();
            let app = adw::Application::builder()
                .application_id(APP_ID)
                .flags(gio::ApplicationFlags::NON_UNIQUE)
                .build();

            let app_id_c = app_id.clone();

            app.connect_activate(move |app| {
                setup_css();
                setup_icons();

                let app_id = app_id_c.clone();

                let launcher = ui::launcher::LauncherWindow::new(app, &app_id, None, move |id| {
                    println!("[Preview] Countdown elapsed or Launch clicked for App ID: {}", id);
                });
                launcher.present();
            });

            app.run_with_args::<&str>(&[]);
        }
        launch::LaunchMode::Manager => {
            let app = adw::Application::builder()
                .application_id(APP_ID)
                .flags(gio::ApplicationFlags::NON_UNIQUE)
                .build();

            app.connect_activate(move |app| {
                setup_css();
                setup_icons();

                let manager = ui::manager::ManagerWindow::new(app, None, None);
                manager.present();
            });

            app.run_with_args::<&str>(&[]);
        }
    }
}
