mod actions;
mod env_vars;
mod managed;
mod state;

use crate::config::{
    GameConfig, ManagedFeatures, find_steam_game_icon, find_steam_game_name, get_config_dir, load_all_configs,
};
use adw::prelude::*;
use gtk4::gdk;
use gtk4::gio;
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
pub struct ManagerWindow {
    pub(super) window: adw::ApplicationWindow,
    pub(super) configs: Rc<RefCell<Vec<GameConfig>>>,
    pub(super) disk_configs: Rc<RefCell<Vec<GameConfig>>>,
    pub(super) selected_idx: Rc<RefCell<Option<usize>>>,
    pub(super) current_managed: Rc<RefCell<Option<ManagedFeatures>>>,
    pub(super) sidebar_list: gtk4::ListBox,
    pub(super) content_box: gtk4::Box,
    pub(super) window_title: adw::WindowTitle,
    pub(super) save_button: gtk4::Button,
    pub(super) revert_button: gtk4::Button,
    pub(super) launch_button: gtk4::Button,
    pub(super) content_toolbar_view: adw::ToolbarView,
    pub(super) on_launch: Option<Rc<dyn Fn()>>,
    pub(super) launching_app_id: Option<String>,
    pub(super) sidebar_dirty_labels: Rc<RefCell<Vec<gtk4::Label>>>,
    pub(super) inherited_env_expanded: Rc<RefCell<bool>>,
    pub(super) force_close: Rc<RefCell<bool>>,
}

impl ManagerWindow {
    pub(super) fn ensure_app_config(loaded: &mut Vec<GameConfig>, app_id: &str) {
        let trimmed = app_id.trim();
        if !trimmed.is_empty() && trimmed != "global" && !loaded.iter().any(|c| c.id == trimmed) {
            let title = find_steam_game_name(trimmed).unwrap_or_else(|| format!("Game {}", trimmed));
            let icon_path = find_steam_game_icon(trimmed);
            let path = get_config_dir().join(format!("{}.env", trimmed));
            loaded.push(GameConfig {
                id: trimmed.to_string(),
                title,
                path,
                icon_path,
                entries: Vec::new(),
            });
        }
    }

    pub fn new(app: &adw::Application, initial_app_id: Option<&str>, on_launch: Option<Rc<dyn Fn()>>) -> Self {
        let launching_app_id = if on_launch.is_some() {
            initial_app_id
                .map(str::trim)
                .filter(|id| !id.is_empty() && *id != "global")
                .map(str::to_string)
        } else {
            None
        };

        let mut loaded = load_all_configs();

        if let Some(app_id) = initial_app_id {
            Self::ensure_app_config(&mut loaded, app_id);
        }

        let configs = Rc::new(RefCell::new(loaded.clone()));
        let disk_configs = Rc::new(RefCell::new(loaded));
        let selected_idx = Rc::new(RefCell::new(None));
        let current_managed = Rc::new(RefCell::new(None));
        let sidebar_dirty_labels = Rc::new(RefCell::new(Vec::new()));
        let inherited_env_expanded = Rc::new(RefCell::new(false));
        let force_close = Rc::new(RefCell::new(false));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Neutron")
            .default_width(880)
            .default_height(660)
            .build();

        let split_view = adw::NavigationSplitView::new();
        split_view.set_min_sidebar_width(280.0);
        split_view.set_max_sidebar_width(340.0);

        // --- Sidebar (Sidebar Page) ---
        let sidebar_toolbar_view = adw::ToolbarView::new();
        let sidebar_header = adw::HeaderBar::new();
        sidebar_header.set_show_end_title_buttons(false);

        let refresh_btn = gtk4::Button::from_icon_name("view-refresh-symbolic");
        refresh_btn.set_tooltip_text(Some("Reload from disk"));
        refresh_btn.add_css_class("circular");
        sidebar_header.pack_end(&refresh_btn);

        sidebar_toolbar_view.add_top_bar(&sidebar_header);

        let sidebar_scrolled = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .build();

        let sidebar_content_box = gtk4::Box::builder().orientation(gtk4::Orientation::Vertical).build();

        let sidebar_list = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .vexpand(false)
            .css_classes(["navigation-sidebar"])
            .build();

        let add_game_btn = gtk4::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("Add game configuration")
            .css_classes(["flat", "circular", "add-game-subtle"])
            .halign(gtk4::Align::Center)
            .hexpand(false)
            .margin_top(4)
            .margin_bottom(10)
            .build();

        sidebar_content_box.append(&sidebar_list);
        sidebar_content_box.append(&add_game_btn);

        sidebar_scrolled.set_child(Some(&sidebar_content_box));
        sidebar_toolbar_view.set_content(Some(&sidebar_scrolled));

        let sidebar_page = adw::NavigationPage::builder()
            .title("Configs")
            .child(&sidebar_toolbar_view)
            .build();
        split_view.set_sidebar(Some(&sidebar_page));

        // --- Content (Detail Page) ---
        let content_toolbar_view = adw::ToolbarView::new();
        let content_header = adw::HeaderBar::new();

        let window_title = adw::WindowTitle::builder().title("Select a configuration").build();
        content_header.set_title_widget(Some(&window_title));

        content_toolbar_view.add_top_bar(&content_header);

        // --- Bottom Action Bar ---
        // Layout: revert | save | launch (launch hidden when unavailable, e.g. Global in standalone mode)
        let bottom_bar = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .homogeneous(true)
            .spacing(10)
            .hexpand(true)
            .css_classes(["toolbar", "bottom-action-bar"])
            .build();

        // 1. Revert button
        let revert_button = gtk4::Button::builder()
            .tooltip_text("Revert changes and reload from disk")
            .sensitive(false)
            .hexpand(true)
            .build();
        let revert_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        revert_box.set_halign(gtk4::Align::Center);
        revert_box.append(&gtk4::Image::from_icon_name("document-revert-symbolic"));
        revert_box.append(&gtk4::Label::new(Some("Revert")));
        revert_button.set_child(Some(&revert_box));

        // 2. Save button
        let save_button = gtk4::Button::builder()
            .tooltip_text("Save configuration to disk")
            .css_classes(["suggested-action"])
            .sensitive(false)
            .hexpand(true)
            .build();
        let save_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        save_box.set_halign(gtk4::Align::Center);
        save_box.append(&gtk4::Image::from_icon_name("document-save-symbolic"));
        save_box.append(&gtk4::Label::new(Some("Save")));
        save_button.set_child(Some(&save_box));

        // 3. Launch button
        let launch_button = gtk4::Button::builder()
            .visible(false)
            .sensitive(false)
            .hexpand(true)
            .build();
        let launch_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        launch_box.set_halign(gtk4::Align::Center);
        launch_box.append(&gtk4::Image::from_icon_name("media-playback-start-symbolic"));
        launch_box.append(&gtk4::Label::new(Some("Launch")));
        launch_button.set_child(Some(&launch_box));

        bottom_bar.append(&revert_button);
        bottom_bar.append(&save_button);
        bottom_bar.append(&launch_button);

        content_toolbar_view.add_bottom_bar(&bottom_bar);
        content_toolbar_view.set_reveal_bottom_bars(false);

        let content_scrolled = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .build();

        let content_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .margin_top(16)
            .margin_bottom(24)
            .margin_start(24)
            .margin_end(24)
            .spacing(18)
            .build();

        content_scrolled.set_child(Some(&content_box));
        content_toolbar_view.set_content(Some(&content_scrolled));

        let content_page = adw::NavigationPage::builder()
            .title("Details")
            .child(&content_toolbar_view)
            .build();
        split_view.set_content(Some(&content_page));

        window.set_content(Some(&split_view));

        let mgr = Self {
            window,
            configs,
            disk_configs,
            selected_idx,
            current_managed,
            sidebar_list,
            content_box,
            window_title,
            save_button,
            revert_button,
            launch_button,
            content_toolbar_view,
            on_launch,
            launching_app_id,
            sidebar_dirty_labels,
            inherited_env_expanded,
            force_close,
        };

        mgr.init(add_game_btn, refresh_btn, initial_app_id);
        mgr
    }

    pub fn present(&self) {
        self.window.present();
    }

    fn init(&self, add_btn: gtk4::Button, refresh_btn: gtk4::Button, initial_app_id: Option<&str>) {
        let mgr = self.clone();

        // Refresh button
        {
            let mgr = mgr.clone();
            refresh_btn.connect_clicked(move |_| {
                let current_id = mgr
                    .selected_idx
                    .borrow()
                    .and_then(|i| mgr.configs.borrow().get(i).map(|c| c.id.clone()));
                *mgr.selected_idx.borrow_mut() = None;
                *mgr.current_managed.borrow_mut() = None;

                let mut loaded = load_all_configs();
                if let Some(ref app_id) = mgr.launching_app_id {
                    Self::ensure_app_config(&mut loaded, app_id);
                }
                *mgr.configs.borrow_mut() = loaded.clone();
                *mgr.disk_configs.borrow_mut() = loaded;
                mgr.populate_sidebar(current_id.as_deref());
            });
        }

        // Add game button
        {
            let mgr = mgr.clone();
            add_btn.connect_clicked(move |_| {
                mgr.prompt_add_game();
            });
        }

        // Sidebar selection change
        {
            let mgr = mgr.clone();
            self.sidebar_list.connect_row_selected(move |_, row| {
                if let Some(row) = row {
                    let idx = row.index() as usize;
                    mgr.select_config(idx);
                }
            });
        }

        // Save button
        {
            let mgr = mgr.clone();
            self.save_button.connect_clicked(move |_| {
                mgr.save_current_config();
            });
        }

        // Revert button
        {
            let mgr = mgr.clone();
            self.revert_button.connect_clicked(move |_| {
                mgr.revert_current_config();
            });
        }

        // Launch button
        {
            let mgr = mgr.clone();
            self.launch_button.connect_clicked(move |_| {
                mgr.launch_current_game();
            });
        }

        // Close request handling with unsaved changes prompt
        {
            let mgr = mgr.clone();
            self.window
                .connect_close_request(move |_| actions::handle_close_request(&mgr));
        }

        // Populate sidebar
        self.populate_sidebar(initial_app_id);
    }

    pub(super) fn populate_sidebar(&self, select_id: Option<&str>) {
        while let Some(row) = self.sidebar_list.first_child() {
            self.sidebar_list.remove(&row);
        }
        self.sidebar_dirty_labels.borrow_mut().clear();

        let configs = self.configs.borrow();
        let mut target_row = None;

        for (idx, cfg) in configs.iter().enumerate() {
            let is_launching_game = self.launching_app_id.as_deref() == Some(cfg.id.as_str());
            let row = adw::ActionRow::builder().title(&cfg.title).build();
            if is_launching_game {
                row.add_css_class("accent");
            }

            if cfg.id == "global" {
                let img = gtk4::Image::from_icon_name("applications-system-symbolic");
                img.set_pixel_size(24);
                row.add_prefix(&img);
            } else {
                let mut loaded_icon = false;
                if let Some(icon_path) = &cfg.icon_path {
                    if let Ok(texture) = gdk::Texture::from_file(&gio::File::for_path(icon_path)) {
                        let img = gtk4::Image::from_paintable(Some(&texture));
                        img.set_pixel_size(28);
                        img.add_css_class("game-icon");
                        row.add_prefix(&img);
                        loaded_icon = true;
                    }
                }
                if !loaded_icon {
                    let img = gtk4::Image::from_icon_name("input-gaming-symbolic");
                    img.set_pixel_size(24);
                    row.add_prefix(&img);
                }
                row.set_subtitle(&format!("App ID: {}", cfg.id));
            }

            let dirty_dot = gtk4::Label::builder()
                .label("●")
                .css_classes(["dirty-dot"])
                .visible(false)
                .valign(gtk4::Align::Center)
                .build();
            row.add_suffix(&dirty_dot);
            self.sidebar_dirty_labels.borrow_mut().push(dirty_dot);

            if is_launching_game && self.on_launch.is_some() {
                let play_btn = gtk4::Button::builder()
                    .icon_name("media-playback-start-symbolic")
                    .tooltip_text(format!("Launch {}", cfg.title))
                    .css_classes(["flat", "circular", "accent"])
                    .valign(gtk4::Align::Center)
                    .build();
                let mgr_play = self.clone();
                play_btn.connect_clicked(move |_| {
                    actions::handle_prelaunch_launch(&mgr_play);
                });
                row.add_suffix(&play_btn);
            }

            if let Some(sid) = select_id {
                if cfg.id == sid {
                    target_row = Some(idx);
                }
            } else if idx == 0 && target_row.is_none() {
                target_row = Some(0);
            }

            self.sidebar_list.append(&row);
        }
        drop(configs);

        if let Some(target) = target_row {
            if let Some(row) = self.sidebar_list.row_at_index(target as i32) {
                self.sidebar_list.select_row(Some(&row));
            }
        }

        self.update_dirty_state();
    }

    pub(super) fn activate_config_managed(&self, idx: usize) {
        let mut configs = self.configs.borrow_mut();
        let Some(cfg) = configs.get_mut(idx) else {
            *self.current_managed.borrow_mut() = None;
            return;
        };
        let is_global = cfg.id == "global";
        let managed = ManagedFeatures::extract(&cfg.entries, is_global);
        cfg.entries.retain(|e| !ManagedFeatures::is_managed_key(e.key.trim()));
        *self.current_managed.borrow_mut() = Some(managed);
    }

    pub(super) fn flush_selected_managed(&self) {
        let prev_selected = *self.selected_idx.borrow();
        if let Some(prev_idx) = prev_selected {
            let mut configs = self.configs.borrow_mut();
            if let Some(cfg) = configs.get_mut(prev_idx) {
                let is_prev_global = cfg.id == "global";
                if let Some(mut mf) = self.current_managed.borrow().clone() {
                    mf.sync_from_entries(&cfg.entries);
                    mf.apply_to_entries(&mut cfg.entries, is_prev_global);
                }
            }
        }
    }

    pub(super) fn select_config(&self, idx: usize) {
        let configs_len = self.configs.borrow().len();
        if idx >= configs_len {
            return;
        }

        let prev_selected = *self.selected_idx.borrow();
        if prev_selected == Some(idx) {
            return;
        }

        // Flush current managed features to previously selected config if needed
        self.flush_selected_managed();
        *self.inherited_env_expanded.borrow_mut() = false;

        *self.selected_idx.borrow_mut() = Some(idx);
        self.activate_config_managed(idx);
        let title = self.configs.borrow()[idx].title.clone();

        // Header WindowTitle: display only the title (no icon, no file path)
        self.window_title.set_title(&title);
        self.window_title.set_subtitle("");

        self.render_editor(idx);
        self.update_dirty_state();
    }

    pub(super) fn render_editor(&self, idx: usize) {
        while let Some(child) = self.content_box.first_child() {
            self.content_box.remove(&child);
        }

        // Managed Configuration Groups (1. Proton version, 2. Wrappers, 3. Headroom)
        for group in managed::build_managed_groups(self, idx) {
            self.content_box.append(&group);
        }

        // Environment Variables Group
        let env_group = env_vars::build_env_group(self, idx);
        self.content_box.append(&env_group);
    }

    pub(super) fn update_dirty_state(&self) {
        state::update_dirty_state(self);
    }

    pub(super) fn save_current_config(&self) {
        actions::save_current_config(self);
    }

    pub(super) fn revert_current_config(&self) {
        actions::revert_current_config(self);
    }

    pub(super) fn launch_current_game(&self) {
        actions::handle_launch_request(self);
    }

    pub(super) fn prompt_add_game(&self) {
        actions::prompt_add_game(self);
    }
}
