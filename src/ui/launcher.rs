use crate::config::{
    compute_effective_envvars, find_steam_game_header, find_steam_game_icon, find_steam_game_name, get_launcher_timeout,
};
use crate::ui::manager::ManagerWindow;
use adw::prelude::*;
use gtk4::glib;
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub struct LauncherWindow {
    window: adw::ApplicationWindow,
    _app_id: String,
    _game_title: String,
    _timer_source: Rc<RefCell<Option<glib::SourceId>>>,
}

impl LauncherWindow {
    pub fn new(
        app: &adw::Application,
        app_id: &str,
        custom_title: Option<&str>,
        on_launch: impl Fn(&str) + 'static,
    ) -> Self {
        let title = custom_title
            .map(|s| s.to_string())
            .or_else(|| {
                if !app_id.trim().is_empty() {
                    find_steam_game_name(app_id)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| {
                if !app_id.trim().is_empty() {
                    format!("Game {}", app_id)
                } else {
                    "Game".to_string()
                }
            });

        let header_texture = find_steam_game_header(app_id)
            .and_then(|path| gtk4::gdk::Texture::from_file(&gtk4::gio::File::for_path(&path)).ok());
        let has_header = header_texture.is_some();

        const WINDOW_WIDTH: i32 = 460;

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title(format!("Starting {}", title))
            .default_width(WINDOW_WIDTH)
            .resizable(false)
            .build();
        window.set_size_request(WINDOW_WIDTH, -1);

        let toolbar_view = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        header.set_show_end_title_buttons(true);

        let window_title = adw::WindowTitle::builder()
            .title(&title)
            .subtitle(if !app_id.trim().is_empty() {
                format!("Steam App ID: {}", app_id)
            } else {
                "Steam Game".to_string()
            })
            .build();
        header.set_title_widget(Some(&window_title));

        toolbar_view.add_top_bar(&header);

        let main_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(16)
            .margin_top(if has_header { 12 } else { 20 })
            .margin_bottom(20)
            .margin_start(24)
            .margin_end(24)
            .valign(gtk4::Align::Start)
            .build();

        if let Some(texture) = header_texture {
            let picture = gtk4::Picture::for_paintable(&texture);
            picture.set_keep_aspect_ratio(true);
            picture.set_can_shrink(true);
            picture.set_height_request(190);
            picture.add_css_class("card");
            picture.add_css_class("game-header-banner");
            main_box.append(&picture);
        } else {
            // Game icon and header fallback
            let title_box = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(14)
                .halign(gtk4::Align::Center)
                .build();

            let icon = if let Some(path) = find_steam_game_icon(app_id) {
                if let Ok(texture) = gtk4::gdk::Texture::from_file(&gtk4::gio::File::for_path(&path)) {
                    let img = gtk4::Image::from_paintable(Some(&texture));
                    img.set_pixel_size(44);
                    img.add_css_class("game-icon");
                    img
                } else {
                    let img = gtk4::Image::from_icon_name("input-gaming-symbolic");
                    img.set_pixel_size(40);
                    img
                }
            } else {
                let img = gtk4::Image::from_icon_name("input-gaming-symbolic");
                img.set_pixel_size(40);
                img
            };

            let label_box = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .valign(gtk4::Align::Center)
                .build();

            let name_label = gtk4::Label::builder()
                .label(&title)
                .css_classes(["title-2"])
                .halign(gtk4::Align::Start)
                .build();

            let id_label = gtk4::Label::builder()
                .label(format!("Steam App ID: {}", app_id))
                .css_classes(["caption", "dim-label"])
                .halign(gtk4::Align::Start)
                .build();

            label_box.append(&name_label);
            label_box.append(&id_label);

            title_box.append(&icon);
            title_box.append(&label_box);
            main_box.append(&title_box);
        }

        // Progress bar and countdown label
        let progress_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .build();

        let total_duration = get_launcher_timeout();
        let total_secs = total_duration.as_secs_f32();

        let countdown_label = gtk4::Label::builder()
            .label(&if total_duration.is_zero() {
                "Launching now...".to_string()
            } else {
                format!("Launching in {:.1}s...", total_secs)
            })
            .css_classes(["body", "dim-label"])
            .halign(gtk4::Align::Center)
            .build();

        let progress_bar = gtk4::ProgressBar::builder().fraction(1.0).hexpand(true).build();

        progress_box.append(&countdown_label);
        progress_box.append(&progress_bar);
        main_box.append(&progress_box);

        // Collapsed element for provided environment variables
        let env_vars = compute_effective_envvars(app_id);

        let env_expander_group = adw::PreferencesGroup::builder().build();

        let env_expander_row = adw::ExpanderRow::builder()
            .title("Environment variables")
            .expanded(false)
            .build();
        env_expander_row.add_css_class("compact-expander");

        let pref_row = adw::PreferencesRow::builder()
            .activatable(false)
            .selectable(false)
            .focusable(false)
            .build();
        let vbox = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .margin_top(8)
            .margin_bottom(10)
            .margin_start(12)
            .margin_end(12)
            .build();

        if env_vars.is_empty() {
            let empty_label = gtk4::Label::builder()
                .label("No environment variables configured")
                .css_classes(["dim-label"])
                .halign(gtk4::Align::Start)
                .build();
            vbox.append(&empty_label);
        } else {
            let grid = gtk4::Grid::builder()
                .column_homogeneous(true)
                .column_spacing(12)
                .row_spacing(8)
                .margin_top(4)
                .margin_bottom(4)
                .build();

            let key_hdr = gtk4::Label::builder()
                .label("VARIABLE")
                .halign(gtk4::Align::Start)
                .hexpand(true)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .css_classes(["heading", "dim-label", "table-header"])
                .build();
            let val_hdr = gtk4::Label::builder()
                .label("VALUE")
                .halign(gtk4::Align::Start)
                .hexpand(true)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .css_classes(["heading", "dim-label", "table-header"])
                .build();

            grid.attach(&key_hdr, 0, 0, 1, 1);
            grid.attach(&val_hdr, 1, 0, 1, 1);

            let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
            grid.attach(&sep, 0, 1, 2, 1);

            for (row_idx, (k, v)) in env_vars.iter().enumerate() {
                let key_label = gtk4::Label::builder()
                    .label(k)
                    .halign(gtk4::Align::Start)
                    .valign(gtk4::Align::Center)
                    .hexpand(true)
                    .selectable(true)
                    .ellipsize(gtk4::pango::EllipsizeMode::End)
                    .tooltip_text(k)
                    .css_classes(["monospace"])
                    .build();
                let val_label = gtk4::Label::builder()
                    .label(v)
                    .halign(gtk4::Align::Start)
                    .valign(gtk4::Align::Center)
                    .hexpand(true)
                    .selectable(true)
                    .ellipsize(gtk4::pango::EllipsizeMode::End)
                    .tooltip_text(v)
                    .css_classes(["monospace"])
                    .build();

                grid.attach(&key_label, 0, (row_idx + 2) as i32, 1, 1);
                grid.attach(&val_label, 1, (row_idx + 2) as i32, 1, 1);
            }

            let scrolled = gtk4::ScrolledWindow::builder()
                .hscrollbar_policy(gtk4::PolicyType::Never)
                .vscrollbar_policy(gtk4::PolicyType::Automatic)
                .max_content_height(480)
                .propagate_natural_height(true)
                .child(&grid)
                .build();

            vbox.append(&scrolled);
        }

        pref_row.set_child(Some(&vbox));
        env_expander_row.add_row(&pref_row);
        env_expander_group.add(&env_expander_row);
        main_box.append(&env_expander_group);

        // Action Buttons
        let button_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(12)
            .homogeneous(true)
            .hexpand(true)
            .margin_top(4)
            .build();

        let config_content = adw::ButtonContent::builder()
            .icon_name("emblem-system-symbolic")
            .label("Configure (F2)")
            .build();

        let config_btn = gtk4::Button::builder().child(&config_content).hexpand(true).build();

        let launch_content = adw::ButtonContent::builder()
            .icon_name("media-playback-start-symbolic")
            .label("Launch Now")
            .build();

        let launch_btn = gtk4::Button::builder().child(&launch_content).hexpand(true).build();

        button_box.append(&config_btn);
        button_box.append(&launch_btn);
        main_box.append(&button_box);

        toolbar_view.set_content(Some(&main_box));
        window.set_content(Some(&toolbar_view));

        let timer_source: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
        let on_launch = Rc::new(on_launch);
        let start_time = Instant::now();

        // Launch helper closure
        let trigger_launch = {
            let window = window.clone();
            let timer_source = timer_source.clone();
            let on_launch = on_launch.clone();
            let app_id = app_id.to_string();

            move || {
                if let Some(source) = timer_source.borrow_mut().take() {
                    source.remove();
                }
                window.close();
                on_launch(&app_id);
            }
        };

        let on_launch_for_config = on_launch.clone();
        let app_id_for_config = app_id.to_string();

        // Open config helper closure
        let open_config = {
            let app = app.clone();
            let window = window.clone();
            let timer_source = timer_source.clone();
            let app_id = app_id.to_string();

            move || {
                if let Some(source) = timer_source.borrow_mut().take() {
                    source.remove();
                }
                window.close();

                // Open Manager focused on this game with launch callback
                let cb = on_launch_for_config.clone();
                let aid = app_id_for_config.clone();
                let mgr = ManagerWindow::new(
                    &app,
                    Some(&app_id),
                    Some(Rc::new(move || {
                        cb(&aid);
                    })),
                );
                mgr.present();
            }
        };

        // Connect button signals
        {
            let open_config = open_config.clone();
            config_btn.connect_clicked(move |_| {
                open_config();
            });
        }
        {
            let trigger_launch = trigger_launch.clone();
            launch_btn.connect_clicked(move |_| {
                trigger_launch();
            });
        }

        // Keyboard controller for F2, Space, Enter, Escape
        let key_controller = gtk4::EventControllerKey::new();
        {
            let open_config = open_config.clone();
            let trigger_launch = trigger_launch.clone();
            let window = window.clone();
            let timer_source = timer_source.clone();

            key_controller.connect_key_pressed(move |_, keyval, _, _| match keyval {
                gtk4::gdk::Key::F2 => {
                    open_config();
                    glib::Propagation::Stop
                }
                gtk4::gdk::Key::Return | gtk4::gdk::Key::KP_Enter | gtk4::gdk::Key::space => {
                    trigger_launch();
                    glib::Propagation::Stop
                }
                gtk4::gdk::Key::Escape => {
                    if let Some(source) = timer_source.borrow_mut().take() {
                        source.remove();
                    }
                    window.close();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            });
        }
        window.add_controller(key_controller);

        // Click anywhere in window (except buttons) to stop the countdown timeout
        let click_controller = gtk4::GestureClick::new();
        click_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
        {
            let timer_source = timer_source.clone();
            let countdown_label = countdown_label.clone();
            let window_ref = window.clone();

            click_controller.connect_pressed(move |_, _n_press, x, y| {
                if let Some(target) = window_ref.pick(x, y, gtk4::PickFlags::DEFAULT) {
                    let mut curr = Some(target);
                    let mut is_button = false;
                    while let Some(w) = curr {
                        if w.downcast_ref::<gtk4::Button>().is_some() {
                            is_button = true;
                            break;
                        }
                        curr = w.parent();
                    }
                    if !is_button {
                        if let Some(source) = timer_source.borrow_mut().take() {
                            source.remove();
                            countdown_label.set_label("Launch paused");
                        }
                    }
                }
            });
        }
        window.add_controller(click_controller);

        // If expander is opened, pause the countdown timeout and fit window height to content
        {
            let timer_source = timer_source.clone();
            let countdown_label = countdown_label.clone();
            let window_clone = window.clone();
            env_expander_row.connect_expanded_notify(move |row| {
                if row.is_expanded() {
                    if let Some(source) = timer_source.borrow_mut().take() {
                        source.remove();
                        countdown_label.set_label("Launch paused");
                    }
                }
                window_clone.set_default_size(WINDOW_WIDTH, -1);
                window_clone.queue_resize();
            });
        }

        // Timer or immediate launch
        if total_duration.is_zero() {
            let trigger_launch = trigger_launch.clone();
            glib::idle_add_local_once(move || {
                trigger_launch();
            });
        } else {
            let timer_source_clone = timer_source.clone();
            let trigger_launch = trigger_launch.clone();
            let countdown_label = countdown_label.clone();
            let progress_bar = progress_bar.clone();

            let source_id = glib::timeout_add_local(Duration::from_millis(20), move || {
                let elapsed = start_time.elapsed();
                if elapsed >= total_duration {
                    countdown_label.set_label("Launching now...");
                    progress_bar.set_fraction(0.0);
                    trigger_launch();
                    glib::ControlFlow::Break
                } else {
                    let remaining = total_duration - elapsed;
                    let remaining_secs = remaining.as_secs_f32();
                    countdown_label.set_label(&format!("Launching in {:.1}s...", remaining_secs));

                    let fraction = (remaining_secs / total_secs).clamp(0.0, 1.0);
                    progress_bar.set_fraction(fraction as f64);

                    glib::ControlFlow::Continue
                }
            });

            *timer_source_clone.borrow_mut() = Some(source_id);
        }

        Self {
            window,
            _app_id: app_id.to_string(),
            _game_title: title,
            _timer_source: timer_source,
        }
    }

    pub fn present(&self) {
        self.window.present();
    }
}
