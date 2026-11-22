use super::{ManagerWindow, state};
use crate::config::{GameConfig, get_config_dir, load_all_configs};
use adw::prelude::*;
use gtk4::glib;
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;

pub fn save_config_at(mgr: &ManagerWindow, idx: usize) {
    let mut to_save = mgr.configs.borrow()[idx].clone();
    let is_global = to_save.id == "global";
    let is_selected = *mgr.selected_idx.borrow() == Some(idx);

    if is_selected {
        if let Some(mut mf) = mgr.current_managed.borrow().clone() {
            mf.sync_from_entries(&to_save.entries);
            mf.apply_to_entries(&mut to_save.entries, is_global);
        }
    } else {
        let mf = crate::config::ManagedFeatures::extract(&to_save.entries, is_global);
        mf.apply_to_entries(&mut to_save.entries, is_global);
    }

    to_save.entries.retain(|e| !e.key.trim().is_empty());

    let result = to_save.save();
    match result {
        Ok(()) => {
            mgr.configs.borrow_mut()[idx] = to_save.clone();
            mgr.disk_configs.borrow_mut()[idx] = to_save;
            if is_selected {
                mgr.activate_config_managed(idx);
                mgr.render_editor(idx);
            }
            mgr.update_dirty_state();
        }
        Err(e) => {
            eprintln!("neutron: error saving {}: {}", to_save.path.display(), e);
        }
    }
}

pub fn save_current_config(mgr: &ManagerWindow) {
    let selected_idx = *mgr.selected_idx.borrow();
    if let Some(idx) = selected_idx {
        save_config_at(mgr, idx);
    }
}

pub fn revert_config_at(mgr: &ManagerWindow, idx: usize) {
    let path = mgr.configs.borrow()[idx].path.clone();
    let is_selected = *mgr.selected_idx.borrow() == Some(idx);
    if path.is_file() {
        match GameConfig::load_from_file(&path) {
            Ok(fresh) => {
                mgr.configs.borrow_mut()[idx] = fresh.clone();
                mgr.disk_configs.borrow_mut()[idx] = fresh;
                if is_selected {
                    mgr.activate_config_managed(idx);
                    mgr.render_editor(idx);
                }
                mgr.update_dirty_state();
            }
            Err(e) => {
                eprintln!("neutron: error reverting {}: {}", path.display(), e);
            }
        }
    } else {
        mgr.configs.borrow_mut()[idx].entries.clear();
        mgr.disk_configs.borrow_mut()[idx].entries.clear();
        if is_selected {
            mgr.activate_config_managed(idx);
            mgr.render_editor(idx);
        }
        mgr.update_dirty_state();
    }
}

pub fn revert_current_config(mgr: &ManagerWindow) {
    let selected_idx = *mgr.selected_idx.borrow();
    if let Some(idx) = selected_idx {
        revert_config_at(mgr, idx);
    }
}

#[derive(Clone, Copy)]
enum UnsavedPromptMode {
    Close,
    Launch,
}

fn collect_dirty_indices(mgr: &ManagerWindow) -> Vec<usize> {
    (0..mgr.configs.borrow().len())
        .filter(|&i| state::is_config_dirty(mgr, i))
        .collect()
}

pub fn build_steam_rungameid_uri(app_id: &str) -> Option<String> {
    let trimmed = app_id.trim();
    if trimmed.is_empty() || trimmed == "global" {
        return None;
    }
    Some(format!("steam://rungameid/{}", trimmed))
}

fn perform_launch(mgr: &ManagerWindow) {
    if let Some(launch_cb) = mgr.on_launch.clone() {
        *mgr.force_close.borrow_mut() = true;
        mgr.window.close();
        launch_cb();
        return;
    }

    let selected = *mgr.selected_idx.borrow();
    let Some(idx) = selected else {
        return;
    };
    let (id, title) = {
        let configs = mgr.configs.borrow();
        let Some(cfg) = configs.get(idx) else {
            return;
        };
        (cfg.id.clone(), cfg.title.clone())
    };

    if let Some(uri) = build_steam_rungameid_uri(&id) {
        let cmd = if crate::config::is_command_available("steam") {
            "steam"
        } else {
            "xdg-open"
        };
        if let Err(e) = std::process::Command::new(cmd).arg(&uri).spawn() {
            eprintln!("neutron: failed to launch {}: {}", title, e);
        }
    }
}

fn launch_after_unsaved_check(mgr: &ManagerWindow) {
    let dirty_indices = collect_dirty_indices(mgr);
    if dirty_indices.is_empty() {
        perform_launch(mgr);
        return;
    }

    prompt_unsaved_dialogs(
        mgr.clone(),
        Rc::new(RefCell::new(dirty_indices)),
        UnsavedPromptMode::Launch,
    );
}

pub fn handle_prelaunch_launch(mgr: &ManagerWindow) {
    if mgr.on_launch.is_none() {
        return;
    }
    launch_after_unsaved_check(mgr);
}

pub fn handle_launch_request(mgr: &ManagerWindow) {
    if !state::can_launch(mgr) {
        return;
    }
    launch_after_unsaved_check(mgr);
}

pub fn handle_close_request(mgr: &ManagerWindow) -> glib::Propagation {
    if *mgr.force_close.borrow() {
        return glib::Propagation::Proceed;
    }

    let dirty_indices = collect_dirty_indices(mgr);
    if dirty_indices.is_empty() {
        return glib::Propagation::Proceed;
    }

    prompt_unsaved_dialogs(
        mgr.clone(),
        Rc::new(RefCell::new(dirty_indices)),
        UnsavedPromptMode::Close,
    );
    glib::Propagation::Stop
}

fn prompt_unsaved_dialogs(mgr: ManagerWindow, remaining: Rc<RefCell<Vec<usize>>>, mode: UnsavedPromptMode) {
    if remaining.borrow().is_empty() {
        match mode {
            UnsavedPromptMode::Close => {
                *mgr.force_close.borrow_mut() = true;
                mgr.window.close();
            }
            UnsavedPromptMode::Launch => {
                perform_launch(&mgr);
            }
        }
        return;
    }

    let idx = remaining.borrow_mut().remove(0);
    if !state::is_config_dirty(&mgr, idx) {
        prompt_unsaved_dialogs(mgr, remaining, mode);
        return;
    }

    let title = mgr.configs.borrow()[idx].title.clone();
    let (body_text, save_label, revert_label) = match mode {
        UnsavedPromptMode::Close => (
            "Do you want to save or revert your changes before closing?",
            "Save and close",
            "Revert and close",
        ),
        UnsavedPromptMode::Launch => (
            "Do you want to save or revert your changes before launching?",
            "Save and launch",
            "Revert and launch",
        ),
    };

    let dialog = adw::Dialog::builder()
        .content_width(360)
        .follows_content_size(true)
        .can_close(true)
        .build();

    let content = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(16)
        .margin_top(24)
        .margin_bottom(20)
        .margin_start(24)
        .margin_end(24)
        .build();

    let heading_label = gtk4::Label::builder()
        .label(format!("Save changes to \"{}\"?", title))
        .css_classes(["title-2"])
        .wrap(true)
        .justify(gtk4::Justification::Center)
        .halign(gtk4::Align::Center)
        .build();

    let body_label = gtk4::Label::builder()
        .label(body_text)
        .wrap(true)
        .justify(gtk4::Justification::Center)
        .halign(gtk4::Align::Center)
        .build();

    let actions_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(8)
        .margin_top(6)
        .build();

    let save_btn = gtk4::Button::builder()
        .label(save_label)
        .css_classes(["suggested-action"])
        .hexpand(true)
        .build();
    let revert_btn = gtk4::Button::builder()
        .label(revert_label)
        .css_classes(["destructive-action"])
        .hexpand(true)
        .build();
    let cancel_btn = gtk4::Button::builder().label("Cancel").hexpand(true).build();

    actions_box.append(&save_btn);
    actions_box.append(&revert_btn);
    actions_box.append(&cancel_btn);

    content.append(&heading_label);
    content.append(&body_label);
    content.append(&actions_box);

    dialog.set_child(Some(&content));
    dialog.set_default_widget(Some(&save_btn));
    dialog.set_focus(Some(&save_btn));

    let handled = Rc::new(RefCell::new(false));

    {
        let mgr_c = mgr.clone();
        let remaining_c = remaining.clone();
        let dialog_c = dialog.clone();
        let handled_c = handled.clone();
        save_btn.connect_clicked(move |_| {
            *handled_c.borrow_mut() = true;
            dialog_c.close();
            save_config_at(&mgr_c, idx);
            prompt_unsaved_dialogs(mgr_c.clone(), remaining_c.clone(), mode);
        });
    }

    {
        let mgr_c = mgr.clone();
        let remaining_c = remaining.clone();
        let dialog_c = dialog.clone();
        let handled_c = handled.clone();
        revert_btn.connect_clicked(move |_| {
            *handled_c.borrow_mut() = true;
            dialog_c.close();
            revert_config_at(&mgr_c, idx);
            prompt_unsaved_dialogs(mgr_c.clone(), remaining_c.clone(), mode);
        });
    }

    {
        let mgr_c = mgr.clone();
        let dialog_c = dialog.clone();
        let handled_c = handled.clone();
        cancel_btn.connect_clicked(move |_| {
            *handled_c.borrow_mut() = true;
            dialog_c.close();
            mgr_c.update_dirty_state();
        });
    }

    {
        let mgr_c = mgr.clone();
        let handled_c = handled;
        dialog.connect_closed(move |_| {
            if !*handled_c.borrow() {
                mgr_c.update_dirty_state();
            }
        });
    }

    dialog.present(Some(&mgr.window));
}

pub fn prompt_add_game(mgr: &ManagerWindow) {
    let dialog = adw::AlertDialog::new(
        Some("Add game configuration"),
        Some("Enter the Steam App ID for the game:"),
    );

    let entry = gtk4::Entry::builder()
        .placeholder_text("e.g. 1808500")
        .margin_top(12)
        .margin_bottom(12)
        .build();

    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("add", "Add game");
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);

    let mgr_dialog = mgr.clone();
    let entry = entry.clone();

    dialog.connect_response(None, move |_, response| {
        if response == "add" {
            let app_id = entry.text().trim().to_string();
            if !app_id.is_empty() {
                let config_path = get_config_dir().join(format!("{}.env", app_id));
                let steam_name = crate::config::find_steam_game_name(&app_id);
                let title = steam_name.unwrap_or_else(|| format!("Game {}", app_id));
                let icon_path = crate::config::find_steam_game_icon(&app_id);

                let new_cfg = GameConfig {
                    id: app_id.clone(),
                    title,
                    path: config_path,
                    icon_path,
                    entries: Vec::new(),
                };
                let _ = new_cfg.save();

                *mgr_dialog.selected_idx.borrow_mut() = None;
                *mgr_dialog.current_managed.borrow_mut() = None;
                let mut loaded = load_all_configs();
                if let Some(ref launching_id) = mgr_dialog.launching_app_id {
                    ManagerWindow::ensure_app_config(&mut loaded, launching_id);
                }
                *mgr_dialog.configs.borrow_mut() = loaded.clone();
                *mgr_dialog.disk_configs.borrow_mut() = loaded;
                mgr_dialog.populate_sidebar(Some(&app_id));
            }
        }
    });

    dialog.present(Some(&mgr.window));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_steam_rungameid_uri() {
        assert_eq!(
            build_steam_rungameid_uri("3669870"),
            Some("steam://rungameid/3669870".to_string())
        );
        assert_eq!(
            build_steam_rungameid_uri("  1808500  "),
            Some("steam://rungameid/1808500".to_string())
        );
        assert_eq!(build_steam_rungameid_uri("global"), None);
        assert_eq!(build_steam_rungameid_uri(""), None);
        assert_eq!(build_steam_rungameid_uri("   "), None);
    }
}
