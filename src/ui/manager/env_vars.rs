use super::ManagerWindow;
use crate::config::{EnvEntry, GameConfig, ManagedFeatures, is_key_overridden_in_entries};
use adw::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;

struct InheritedRowWidgets {
    key: String,
    value: String,
    key_label: gtk4::Label,
    val_label: gtk4::Label,
}

fn wrap_key_entry_with_check(key_entry: &gtk4::Entry, check_btn: &gtk4::CheckButton) -> gtk4::Overlay {
    let overlay = gtk4::Overlay::builder()
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    overlay.set_child(Some(key_entry));
    overlay.add_overlay(check_btn);
    overlay
}

fn wrap_val_entry_with_delete(val_entry: &gtk4::Entry, delete_btn: &gtk4::Button) -> gtk4::Overlay {
    let overlay = gtk4::Overlay::builder()
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    overlay.set_child(Some(val_entry));
    overlay.add_overlay(delete_btn);
    overlay
}

fn build_check_button(active: bool) -> gtk4::CheckButton {
    gtk4::CheckButton::builder()
        .active(active)
        .can_target(false)
        .focusable(false)
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .margin_end(7)
        .css_classes(["env-key-check"])
        .build()
}

fn build_delete_button() -> gtk4::Button {
    gtk4::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text("Delete override")
        .css_classes(["flat", "circular", "env-val-delete"])
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .margin_end(5)
        .can_target(false)
        .focusable(false)
        .build()
}

fn apply_disabled_style(
    key_entry: &gtk4::Entry,
    val_entry: &gtk4::Entry,
    check_btn: &gtk4::CheckButton,
    enabled: bool,
) {
    let attrs = gtk4::pango::AttrList::new();
    if enabled {
        key_entry.remove_css_class("disabled-entry");
        val_entry.remove_css_class("disabled-entry");
        check_btn.set_tooltip_text(Some("Disable variable"));
    } else {
        attrs.insert(gtk4::pango::AttrInt::new_strikethrough(true));
        key_entry.add_css_class("disabled-entry");
        val_entry.add_css_class("disabled-entry");
        check_btn.set_tooltip_text(Some("Enable variable"));
    }
    key_entry.set_attributes(&attrs);
    val_entry.set_attributes(&attrs);
}

fn update_row_modified_style(
    key_entry: &gtk4::Entry,
    val_entry: &gtk4::Entry,
    check_btn: &gtk4::CheckButton,
    disk_configs: &Rc<RefCell<Vec<GameConfig>>>,
    config_idx: usize,
) {
    let cur_k = key_entry.text().to_string();
    let cur_v = val_entry.text().to_string();
    let cur_enabled = check_btn.is_active();
    let in_disk = disk_configs
        .borrow()
        .get(config_idx)
        .map(|d| {
            d.entries.iter().any(|e| {
                !ManagedFeatures::is_managed_key(e.key.trim())
                    && e.key == cur_k
                    && e.value == cur_v
                    && e.enabled == cur_enabled
            })
        })
        .unwrap_or(false);
    if in_disk {
        key_entry.remove_css_class("modified-entry");
        val_entry.remove_css_class("modified-entry");
    } else {
        key_entry.add_css_class("modified-entry");
        val_entry.add_css_class("modified-entry");
    }
}

fn wire_key_warning(entry: &gtk4::Entry) -> Rc<dyn Fn()> {
    let popover = gtk4::Popover::builder()
        .autohide(false)
        .can_target(false)
        .focusable(false)
        .position(gtk4::PositionType::Top)
        .css_classes(["warning-popover"])
        .build();
    popover.set_parent(entry);

    let label = gtk4::Label::builder()
        .label("This configuration is managed by one of the options above.")
        .css_classes(["warning-popover-label"])
        .build();
    popover.set_child(Some(&label));

    let update_warning: Rc<dyn Fn()> = {
        let entry = entry.clone();
        let popover = popover.clone();
        Rc::new(move || {
            let raw = entry.text().to_string();
            let clean = raw.trim();
            let is_managed = !clean.is_empty()
                && (ManagedFeatures::is_managed_key(clean)
                    || clean.starts_with("NEUTRON_")
                    || clean == "PROTON_DLSS_INDICATOR");
            if is_managed {
                entry.set_icon_from_icon_name(gtk4::EntryIconPosition::Secondary, Some("dialog-warning-symbolic"));
                entry.set_icon_activatable(gtk4::EntryIconPosition::Secondary, false);
                entry.set_icon_sensitive(gtk4::EntryIconPosition::Secondary, true);
                entry.set_icon_tooltip_text(gtk4::EntryIconPosition::Secondary, None);
                entry.add_css_class("warning");
            } else {
                if popover.is_visible() {
                    popover.popdown();
                }
                entry.set_icon_from_icon_name(gtk4::EntryIconPosition::Secondary, None);
                entry.set_icon_tooltip_text(gtk4::EntryIconPosition::Secondary, None);
                entry.remove_css_class("warning");
            }
        })
    };

    let motion = gtk4::EventControllerMotion::new();
    let check_hover: Rc<dyn Fn(f64, f64)> = {
        let entry = entry.clone();
        let popover = popover.clone();
        Rc::new(move |x, y| {
            if entry.icon_name(gtk4::EntryIconPosition::Secondary).is_some() {
                let rect = entry.icon_area(gtk4::EntryIconPosition::Secondary);
                let rx = rect.x();
                let ry = rect.y();
                let rw = rect.width();
                let rh = rect.height();
                if rw > 0 && rh > 0 {
                    let hit = (x as i32) >= rx - 4
                        && (x as i32) <= rx + rw + 4
                        && (y as i32) >= (ry - 6).max(0)
                        && (y as i32) <= ry + rh + 6;
                    if hit {
                        if !popover.is_visible() {
                            popover.set_pointing_to(Some(&rect));
                            popover.popup();
                        }
                        return;
                    }
                }
            }
            if popover.is_visible() {
                popover.popdown();
            }
        })
    };

    {
        let check_hover = check_hover.clone();
        motion.connect_enter(move |_, x, y| {
            check_hover(x, y);
        });
    }
    {
        let check_hover = check_hover;
        motion.connect_motion(move |_, x, y| {
            check_hover(x, y);
        });
    }
    {
        let popover = popover.clone();
        motion.connect_leave(move |_| {
            if popover.is_visible() {
                popover.popdown();
            }
        });
    }
    {
        let popover = popover;
        entry.connect_unmap(move |_| {
            if popover.is_visible() {
                popover.popdown();
            }
        });
    }

    entry.add_controller(motion);

    {
        let uw = update_warning.clone();
        entry.connect_changed(move |_| {
            uw();
        });
    }

    update_warning();
    update_warning
}

pub fn build_env_group(mgr: &ManagerWindow, idx: usize) -> adw::PreferencesGroup {
    let env_group = adw::PreferencesGroup::builder().title("Environment variables").build();

    let card = build_custom_table(mgr, idx);
    env_group.add(&card);

    env_group
}

fn build_custom_table(mgr: &ManagerWindow, idx: usize) -> gtk4::Box {
    let is_global = mgr.configs.borrow()[idx].id == "global";

    let inherited_rows: Rc<RefCell<Vec<InheritedRowWidgets>>> = Rc::new(RefCell::new(Vec::new()));
    let refresh_inherited: Rc<dyn Fn()> = {
        let inherited_rows = inherited_rows.clone();
        let configs = mgr.configs.clone();
        Rc::new(move || {
            let rows = inherited_rows.borrow();
            if rows.is_empty() {
                return;
            }
            let c = configs.borrow();
            let Some(cfg) = c.get(idx) else {
                return;
            };
            let entries = &cfg.entries;
            for row in rows.iter() {
                let escaped_key = gtk4::glib::markup_escape_text(&row.key);
                let escaped_val = gtk4::glib::markup_escape_text(&row.value);
                if is_key_overridden_in_entries(entries, &row.key) {
                    row.key_label.set_markup(&format!("<s>{}</s>", escaped_key));
                    row.val_label.set_markup(&format!("<s>{}</s>", escaped_val));
                    row.key_label.add_css_class("dim-label");
                    row.val_label.add_css_class("dim-label");
                    row.key_label.add_css_class("strikethrough");
                    row.val_label.add_css_class("strikethrough");
                    let tip = format!("'{}' is overridden by game configuration", row.key);
                    row.key_label.set_tooltip_text(Some(&tip));
                    row.val_label.set_tooltip_text(Some(&tip));
                } else {
                    row.key_label.set_markup(&escaped_key);
                    row.val_label.set_markup(&escaped_val);
                    row.key_label.remove_css_class("dim-label");
                    row.val_label.remove_css_class("dim-label");
                    row.key_label.remove_css_class("strikethrough");
                    row.val_label.remove_css_class("strikethrough");
                    row.key_label
                        .set_tooltip_text(Some("Inherited from global configuration"));
                    row.val_label
                        .set_tooltip_text(Some("Inherited from global configuration"));
                }
            }
        })
    };

    let card = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .css_classes(["card", "env-card"])
        .build();

    let grid = gtk4::Grid::builder()
        .column_homogeneous(true)
        .column_spacing(12)
        .row_spacing(8)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(14)
        .margin_end(14)
        .build();

    let key_hdr = gtk4::Label::builder()
        .label("VARIABLE")
        .halign(gtk4::Align::Start)
        .hexpand(true)
        .css_classes(["heading", "dim-label", "table-header"])
        .build();
    let val_hdr = gtk4::Label::builder()
        .label("VALUE")
        .halign(gtk4::Align::Start)
        .hexpand(true)
        .css_classes(["heading", "dim-label", "table-header"])
        .build();

    grid.attach(&key_hdr, 0, 0, 1, 1);
    grid.attach(&val_hdr, 1, 0, 1, 1);

    let h_sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    grid.attach(&h_sep, 0, 1, 2, 1);

    let custom_indices: Vec<usize> = {
        let c = mgr.configs.borrow();
        c[idx]
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !ManagedFeatures::is_managed_key(e.key.trim()))
            .map(|(i, _)| i)
            .collect()
    };

    let mut row_num = 2;
    let configs = mgr.configs.clone();

    for real_idx in custom_indices {
        let (key_val, value_val, enabled_val) = {
            let c = configs.borrow();
            let e = &c[idx].entries[real_idx];
            (e.key.clone(), e.value.clone(), e.enabled)
        };

        let check_btn = build_check_button(enabled_val);

        let key_entry = gtk4::Entry::builder()
            .text(&key_val)
            .placeholder_text("KEY")
            .hexpand(true)
            .css_classes(["monospace", "env-key-entry"])
            .valign(gtk4::Align::Center)
            .build();

        let _ = wire_key_warning(&key_entry);
        let key_overlay = wrap_key_entry_with_check(&key_entry, &check_btn);

        let val_entry = gtk4::Entry::builder()
            .text(&value_val)
            .placeholder_text("VALUE")
            .hexpand(true)
            .css_classes(["monospace", "env-val-entry"])
            .valign(gtk4::Align::Center)
            .build();

        let delete_btn = build_delete_button();
        let val_overlay = wrap_val_entry_with_delete(&val_entry, &delete_btn);

        apply_disabled_style(&key_entry, &val_entry, &check_btn, enabled_val);
        update_row_modified_style(&key_entry, &val_entry, &check_btn, &mgr.disk_configs, idx);

        // Connect in-place edits
        {
            let configs = configs.clone();
            let ke = key_entry.clone();
            let ve = val_entry.clone();
            let mgr = mgr.clone();
            let refresh_inh = refresh_inherited.clone();
            check_btn.connect_toggled(move |btn| {
                let active = btn.is_active();
                {
                    let mut c = configs.borrow_mut();
                    if let Some(entry) = c[idx].entries.get_mut(real_idx) {
                        entry.enabled = active;
                    }
                }
                apply_disabled_style(&ke, &ve, btn, active);
                update_row_modified_style(&ke, &ve, btn, &mgr.disk_configs, idx);
                refresh_inh();
                mgr.update_dirty_state();
            });
        }

        {
            let configs = configs.clone();
            let ke = key_entry.clone();
            let ve = val_entry.clone();
            let cb = check_btn.clone();
            let mgr = mgr.clone();
            let refresh_inh = refresh_inherited.clone();
            key_entry.connect_changed(move |_| {
                {
                    let mut c = configs.borrow_mut();
                    if let Some(entry) = c[idx].entries.get_mut(real_idx) {
                        entry.key = ke.text().to_string();
                    }
                }
                update_row_modified_style(&ke, &ve, &cb, &mgr.disk_configs, idx);
                refresh_inh();
                mgr.update_dirty_state();
            });
        }

        {
            let configs = configs.clone();
            let ke = key_entry.clone();
            let ve = val_entry.clone();
            let cb = check_btn.clone();
            let mgr = mgr.clone();
            let refresh_inh = refresh_inherited.clone();
            val_entry.connect_changed(move |_| {
                {
                    let mut c = configs.borrow_mut();
                    if let Some(entry) = c[idx].entries.get_mut(real_idx) {
                        entry.value = ve.text().to_string();
                    }
                }
                update_row_modified_style(&ke, &ve, &cb, &mgr.disk_configs, idx);
                refresh_inh();
                mgr.update_dirty_state();
            });
        }

        wire_row_controls_visibility(
            &key_overlay,
            &key_entry,
            &check_btn,
            &val_overlay,
            &val_entry,
            &delete_btn,
        );

        {
            let configs = configs.clone();
            let mgr = mgr.clone();
            delete_btn.connect_clicked(move |_| {
                configs.borrow_mut()[idx].entries.remove(real_idx);
                mgr.render_editor(idx);
                mgr.update_dirty_state();
            });
        }

        grid.attach(&key_overlay, 0, row_num, 1, 1);
        grid.attach(&val_overlay, 1, row_num, 1, 1);
        row_num += 1;
    }

    attach_empty_row(&grid, row_num, idx, configs, mgr.clone(), refresh_inherited.clone());

    card.append(&grid);

    // In game-specific configs, add the inherited variables expander at the end of the card block
    if !is_global {
        let global_custom_entries: Vec<EnvEntry> = {
            let configs = mgr.configs.borrow();
            configs
                .iter()
                .find(|c| c.id == "global")
                .map(|c| {
                    c.entries
                        .iter()
                        .filter(|e| {
                            e.enabled && !e.key.trim().is_empty() && !ManagedFeatures::is_managed_key(e.key.trim())
                        })
                        .cloned()
                        .collect()
                })
                .unwrap_or_default()
        };

        if !global_custom_entries.is_empty() {
            let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
            card.append(&sep);

            let count = global_custom_entries.len();
            let expander = gtk4::Expander::builder()
                .expanded(*mgr.inherited_env_expanded.borrow())
                .margin_top(8)
                .margin_bottom(10)
                .margin_start(14)
                .margin_end(14)
                .build();

            {
                let expanded_state = mgr.inherited_env_expanded.clone();
                expander.connect_expanded_notify(move |exp| {
                    *expanded_state.borrow_mut() = exp.is_expanded();
                });
            }

            let header_box = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(6)
                .build();

            let title_label = gtk4::Label::builder()
                .label("Inherited global variables")
                .halign(gtk4::Align::Start)
                .build();

            let count_label = gtk4::Label::builder()
                .label(format!("({} variable{})", count, if count == 1 { "" } else { "s" }))
                .css_classes(["dim-label"])
                .halign(gtk4::Align::Start)
                .build();

            header_box.append(&title_label);
            header_box.append(&count_label);
            expander.set_label_widget(Some(&header_box));

            let inh_grid = gtk4::Grid::builder()
                .column_homogeneous(true)
                .column_spacing(12)
                .row_spacing(8)
                .margin_top(10)
                .margin_bottom(6)
                .build();

            let key_hdr = gtk4::Label::builder()
                .label("VARIABLE")
                .halign(gtk4::Align::Start)
                .hexpand(true)
                .css_classes(["heading", "dim-label", "table-header"])
                .build();
            let val_hdr = gtk4::Label::builder()
                .label("VALUE")
                .halign(gtk4::Align::Start)
                .hexpand(true)
                .css_classes(["heading", "dim-label", "table-header"])
                .build();

            inh_grid.attach(&key_hdr, 0, 0, 1, 1);
            inh_grid.attach(&val_hdr, 1, 0, 1, 1);

            let sub_sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
            inh_grid.attach(&sub_sep, 0, 1, 2, 1);

            for (g_idx, g_entry) in global_custom_entries.iter().enumerate() {
                let key_label = gtk4::Label::builder()
                    .halign(gtk4::Align::Start)
                    .valign(gtk4::Align::Center)
                    .hexpand(true)
                    .selectable(true)
                    .use_markup(true)
                    .css_classes(["monospace"])
                    .build();

                let val_label = gtk4::Label::builder()
                    .halign(gtk4::Align::Start)
                    .valign(gtk4::Align::Center)
                    .hexpand(true)
                    .selectable(true)
                    .use_markup(true)
                    .css_classes(["monospace"])
                    .build();

                inh_grid.attach(&key_label, 0, (g_idx + 2) as i32, 1, 1);
                inh_grid.attach(&val_label, 1, (g_idx + 2) as i32, 1, 1);

                inherited_rows.borrow_mut().push(InheritedRowWidgets {
                    key: g_entry.key.clone(),
                    value: g_entry.value.clone(),
                    key_label,
                    val_label,
                });
            }

            refresh_inherited();

            expander.set_child(Some(&inh_grid));
            card.append(&expander);
        }
    }

    card
}

fn wire_row_controls_visibility(
    key_overlay: &gtk4::Overlay,
    key_entry: &gtk4::Entry,
    check_btn: &gtk4::CheckButton,
    val_overlay: &gtk4::Overlay,
    val_entry: &gtk4::Entry,
    delete_btn: &gtk4::Button,
) {
    let key_focus = gtk4::EventControllerFocus::new();
    let val_focus = gtk4::EventControllerFocus::new();
    let key_motion = gtk4::EventControllerMotion::new();
    let val_motion = gtk4::EventControllerMotion::new();

    key_overlay.add_controller(key_focus.clone());
    val_overlay.add_controller(val_focus.clone());
    key_overlay.add_controller(key_motion.clone());
    val_overlay.add_controller(val_motion.clone());

    let is_key_focused = Rc::new(RefCell::new(
        key_focus.contains_focus() || key_entry.focus_child().is_some() || key_entry.has_focus(),
    ));
    let is_val_focused = Rc::new(RefCell::new(
        val_focus.contains_focus() || val_entry.focus_child().is_some() || val_entry.has_focus(),
    ));
    let is_key_hovered = Rc::new(RefCell::new(key_motion.contains_pointer()));
    let is_val_hovered = Rc::new(RefCell::new(val_motion.contains_pointer()));

    let update_controls = {
        let key_entry = key_entry.clone();
        let check_btn = check_btn.clone();
        let val_entry = val_entry.clone();
        let delete_btn = delete_btn.clone();
        let is_key_focused = is_key_focused.clone();
        let is_val_focused = is_val_focused.clone();
        let is_key_hovered = is_key_hovered.clone();
        let is_val_hovered = is_val_hovered.clone();

        move || {
            let should_show = *is_key_focused.borrow()
                || *is_val_focused.borrow()
                || *is_key_hovered.borrow()
                || *is_val_hovered.borrow();

            if should_show {
                val_entry.add_css_class("show-trash");
                delete_btn.add_css_class("show-trash");
                delete_btn.set_can_target(true);
                delete_btn.set_focusable(true);
                key_entry.add_css_class("show-check");
                check_btn.add_css_class("show-check");
                check_btn.set_can_target(true);
                check_btn.set_focusable(true);
            } else {
                val_entry.remove_css_class("show-trash");
                delete_btn.remove_css_class("show-trash");
                delete_btn.set_can_target(false);
                delete_btn.set_focusable(false);
                key_entry.remove_css_class("show-check");
                check_btn.remove_css_class("show-check");
                check_btn.set_can_target(false);
                check_btn.set_focusable(false);
            }
        }
    };

    {
        let is_key_focused = is_key_focused.clone();
        let update = update_controls.clone();
        key_focus.connect_enter(move |_| {
            *is_key_focused.borrow_mut() = true;
            update();
        });
    }
    {
        let is_key_focused = is_key_focused;
        let update = update_controls.clone();
        key_focus.connect_leave(move |_| {
            *is_key_focused.borrow_mut() = false;
            update();
        });
    }

    {
        let is_val_focused = is_val_focused.clone();
        let update = update_controls.clone();
        val_focus.connect_enter(move |_| {
            *is_val_focused.borrow_mut() = true;
            update();
        });
    }
    {
        let is_val_focused = is_val_focused;
        let update = update_controls.clone();
        val_focus.connect_leave(move |_| {
            *is_val_focused.borrow_mut() = false;
            update();
        });
    }

    {
        let is_key_hovered = is_key_hovered.clone();
        let update = update_controls.clone();
        key_motion.connect_enter(move |_, _, _| {
            *is_key_hovered.borrow_mut() = true;
            update();
        });
    }
    {
        let is_key_hovered = is_key_hovered;
        let update = update_controls.clone();
        key_motion.connect_leave(move |_| {
            *is_key_hovered.borrow_mut() = false;
            update();
        });
    }

    {
        let is_val_hovered = is_val_hovered.clone();
        let update = update_controls.clone();
        val_motion.connect_enter(move |_, _, _| {
            *is_val_hovered.borrow_mut() = true;
            update();
        });
    }
    {
        let is_val_hovered = is_val_hovered;
        let update = update_controls.clone();
        val_motion.connect_leave(move |_| {
            *is_val_hovered.borrow_mut() = false;
            update();
        });
    }

    update_controls();
}

fn attach_empty_row(
    grid: &gtk4::Grid,
    row_num: i32,
    config_idx: usize,
    configs: Rc<RefCell<Vec<GameConfig>>>,
    mgr: ManagerWindow,
    refresh_inherited: Rc<dyn Fn()>,
) {
    let empty_check = build_check_button(true);

    let empty_key = gtk4::Entry::builder()
        .placeholder_text("NEW_VARIABLE")
        .hexpand(true)
        .css_classes(["monospace", "env-key-entry"])
        .valign(gtk4::Align::Center)
        .build();

    let update_empty_warning = wire_key_warning(&empty_key);
    let empty_key_overlay = wrap_key_entry_with_check(&empty_key, &empty_check);

    let empty_val = gtk4::Entry::builder()
        .placeholder_text("VALUE")
        .hexpand(true)
        .css_classes(["monospace", "env-val-entry"])
        .valign(gtk4::Align::Center)
        .build();

    let empty_delete_btn = build_delete_button();
    let empty_val_overlay = wrap_val_entry_with_delete(&empty_val, &empty_delete_btn);

    grid.attach(&empty_key_overlay, 0, row_num, 1, 1);
    grid.attach(&empty_val_overlay, 1, row_num, 1, 1);

    let committed = Rc::new(RefCell::new(false));
    let grid_clone = grid.clone();

    let on_input: Rc<dyn Fn()> = {
        let configs = configs.clone();
        let mgr = mgr.clone();
        let empty_check = empty_check.clone();
        let empty_key = empty_key.clone();
        let empty_key_overlay = empty_key_overlay.clone();
        let empty_val = empty_val.clone();
        let empty_val_overlay = empty_val_overlay.clone();
        let empty_delete_btn = empty_delete_btn.clone();
        let committed = committed.clone();
        let grid = grid_clone;
        let update_empty_warning = update_empty_warning.clone();
        let refresh_inherited = refresh_inherited.clone();

        Rc::new(move || {
            if *committed.borrow() {
                return;
            }

            let k = empty_key.text().to_string();
            let v = empty_val.text().to_string();

            if !k.is_empty() || !v.is_empty() {
                *committed.borrow_mut() = true;

                let new_real_idx = configs.borrow()[config_idx].entries.len();
                configs.borrow_mut()[config_idx].entries.push(EnvEntry {
                    key: k,
                    value: v,
                    enabled: true,
                    comment_above: None,
                });

                apply_disabled_style(&empty_key, &empty_val, &empty_check, true);
                update_row_modified_style(&empty_key, &empty_val, &empty_check, &mgr.disk_configs, config_idx);
                update_empty_warning();

                wire_row_controls_visibility(
                    &empty_key_overlay,
                    &empty_key,
                    &empty_check,
                    &empty_val_overlay,
                    &empty_val,
                    &empty_delete_btn,
                );

                {
                    let configs = configs.clone();
                    let mgr = mgr.clone();
                    empty_delete_btn.connect_clicked(move |_| {
                        configs.borrow_mut()[config_idx].entries.remove(new_real_idx);
                        mgr.render_editor(config_idx);
                        mgr.update_dirty_state();
                    });
                }

                {
                    let configs = configs.clone();
                    let ke = empty_key.clone();
                    let ve = empty_val.clone();
                    let mgr = mgr.clone();
                    let refresh_inh = refresh_inherited.clone();
                    empty_check.connect_toggled(move |btn| {
                        let active = btn.is_active();
                        {
                            let mut c = configs.borrow_mut();
                            if let Some(entry) = c[config_idx].entries.get_mut(new_real_idx) {
                                entry.enabled = active;
                            }
                        }
                        apply_disabled_style(&ke, &ve, btn, active);
                        update_row_modified_style(&ke, &ve, btn, &mgr.disk_configs, config_idx);
                        refresh_inh();
                        mgr.update_dirty_state();
                    });
                }

                {
                    let configs = configs.clone();
                    let ke = empty_key.clone();
                    let ve = empty_val.clone();
                    let cb = empty_check.clone();
                    let mgr = mgr.clone();
                    let refresh_inh = refresh_inherited.clone();
                    empty_key.connect_changed(move |_| {
                        {
                            let mut c = configs.borrow_mut();
                            if let Some(entry) = c[config_idx].entries.get_mut(new_real_idx) {
                                entry.key = ke.text().to_string();
                            }
                        }
                        update_row_modified_style(&ke, &ve, &cb, &mgr.disk_configs, config_idx);
                        refresh_inh();
                        mgr.update_dirty_state();
                    });
                }
                {
                    let configs = configs.clone();
                    let ke = empty_key.clone();
                    let ve = empty_val.clone();
                    let cb = empty_check.clone();
                    let mgr = mgr.clone();
                    let refresh_inh = refresh_inherited.clone();
                    empty_val.connect_changed(move |_| {
                        {
                            let mut c = configs.borrow_mut();
                            if let Some(entry) = c[config_idx].entries.get_mut(new_real_idx) {
                                entry.value = ve.text().to_string();
                            }
                        }
                        update_row_modified_style(&ke, &ve, &cb, &mgr.disk_configs, config_idx);
                        refresh_inh();
                        mgr.update_dirty_state();
                    });
                }

                refresh_inherited();
                mgr.update_dirty_state();
                attach_empty_row(
                    &grid,
                    row_num + 1,
                    config_idx,
                    configs.clone(),
                    mgr.clone(),
                    refresh_inherited.clone(),
                );
            }
        })
    };

    {
        let on_input = on_input.clone();
        empty_key.connect_changed(move |_| on_input());
    }
    {
        let on_input = on_input;
        empty_val.connect_changed(move |_| on_input());
    }
}
