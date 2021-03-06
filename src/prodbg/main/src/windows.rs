extern crate minifb;
extern crate bgfx;
extern crate viewdock;
extern crate renderer;
extern crate nfd;

use minifb::{CursorStyle, Scale, WindowOptions, MouseMode, MouseButton, Key, KeyRepeat};
use renderer::Renderer;
use core::view_plugins::{ViewHandle, ViewPlugins, ViewInstance};
use core::backend_plugin::BackendPlugins;
use core::session::{Sessions, Session, SessionHandle};
use core::reader_wrapper::ReaderWrapper;
use self::viewdock::{Workspace, Rect, Direction, DockHandle, SizerPos, Dock, ItemTarget};
use settings::Settings;
use std::fs::File;
use std::io;
use menu::*;
use imgui_sys::Imgui;
use prodbg_api::ui_ffi::{PDVec2, ImguiKey};
use prodbg_api::view::CViewCallbacks;
use std::os::raw::c_void;
use std::collections::VecDeque;
use std::io::{Read, Write};
use statusbar::Statusbar;
use self::nfd::Response as NfdResponse;
use prodbg_api::events;

// use std::mem::transmute;

const WIDTH: i32 = 1280;
const HEIGHT: i32 = 800;
const WORKSPACE_UNDO_LIMIT: usize = 10;
const OVERLAY_COLOR: u32 = 0x8000FF00;

enum State {
    Default,
    DraggingNothing,
    DraggingSizer(SizerPos),
    DraggingDock(DockHandle),
    PreDraggingDock(DockHandle),
}

pub struct MouseState {
    state: State,
    prev_mouse: (f32, f32),
}

impl MouseState {
    pub fn new() -> MouseState {
        MouseState {
            state: State::Default,
            prev_mouse: (0.0, 0.0),
        }
    }
}

pub struct Window {
    /// minifb window
    pub win: minifb::Window,
    pub menu: Menu,

    /// Views in this window
    pub views: Vec<ViewHandle>,

    pub ws: Workspace,
    ws_states: VecDeque<String>,
    cur_state_index: usize,

    pub mouse_state: MouseState,

    pub menu_id_offset: u32,

    pub overlay: Option<(DockHandle, Rect)>,
    pub context_menu_data: Option<(DockHandle, (f32, f32))>,

    pub statusbar: Statusbar,
}

struct WindowState {
    pub showed_popup: u32,
    pub should_close: bool,
}

struct KeyCharCallback;

impl minifb::InputCallback for KeyCharCallback {
    fn add_char(&mut self, key: u32) {
        Imgui::add_input_character(key as u16);
    }
}

/// ! Windows keeps track of all different windows that are present with in the application
/// ! There are several ways windows can be created:
/// !
/// ! 1. User opens a new window using a shortcut or menu selection.
/// ! 2. User "undocks" a view from an existing window giving it it's own floating window.
/// ! 3. etc

pub struct Windows {
    /// All the windows being tracked
    windows: Vec<Window>,
    current: usize,
    renderer: Renderer,
}

impl Windows {
    pub fn new() -> Windows {
        Windows {
            windows: Vec::new(),
            renderer: Renderer::new(),
            current: 0,
        }
    }

    /// Create a default window which will only be created if there are no other
    pub fn create_default(&mut self, settings: &Settings) {
        if self.windows.len() > 0 {
            return;
        }

        let window = self.create_window_with_menus(settings).expect("Unable to create window");

        Self::setup_imgui_key_mappings();

        self.windows.push(window)
    }

    pub fn create_window(&mut self, width: usize, height: usize) -> minifb::Result<Window> {
        let win = try!(minifb::Window::new("ProDBG",
                                           width,
                                           height,
                                           WindowOptions {
                                               resize: true,
                                               scale: Scale::X1,
                                               ..WindowOptions::default()
                                           }));
        // TODO: Return correctly
        self.renderer.setup_window(win.get_window_handle(), width as u16, height as u16).unwrap();

        let ws = Workspace::new(Rect::new(0.0, 0.0, width as f32, (height - 20) as f32));
        let mut ws_states = VecDeque::with_capacity(WORKSPACE_UNDO_LIMIT);

        ws_states.push_back(ws.save_state());

        return Ok(Window {
            win: win,
            menu: Menu::new(),
            views: Vec::new(),
            menu_id_offset: 1000,
            mouse_state: MouseState::new(),
            ws: ws,
            ws_states: ws_states,
            cur_state_index: 0usize,
            overlay: None,
            context_menu_data: None,
            statusbar: Statusbar::new(),
        });
    }

    pub fn create_window_with_menus(&mut self, settings: &Settings) -> minifb::Result<Window> {

        let width = settings.get_int("window_size", "width").unwrap_or(WIDTH) as usize;
        let height = settings.get_int("window_size", "height").unwrap_or(HEIGHT) as usize;

        let mut window = try!(self.create_window(width, height));

        window.win.set_input_callback(Box::new(KeyCharCallback {}));

        // we ignore the results because we likely brake this on Linux otherwise
        // TODO: Figure out how to deal with this on Linux
        let _ = window.win.add_menu(&window.menu.file_menu);
        let _ = window.win.add_menu(&window.menu.debug_menu);

        Ok(window)
    }

    pub fn update(&mut self,
                  sessions: &mut Sessions,
                  view_plugins: &mut ViewPlugins,
                  backend_plugins: &mut BackendPlugins) {
        for win in &mut self.windows {
            win.pre_update();
        }

        self.renderer.pre_update();

        for i in (0..self.windows.len()).rev() {
            self.windows[i].update(sessions, view_plugins, backend_plugins);
            self.renderer.update_size(self.windows[i].win.get_size());

            if !self.windows[i].win.is_open() {
                // TODO: Support more than one window
                let _ = self.windows[i].save_layout("data/user_layout.json", view_plugins);
                self.windows.swap_remove(i);
            }
        }

        self.renderer.post_update();
    }

    pub fn get_current(&mut self) -> &mut Window {
        let current = self.current;
        &mut self.windows[current]
    }

    /// Checks if application should exit (all window instances closed)
    pub fn should_exit(&self) -> bool {
        self.windows.len() == 0
    }

    /// Save the state of the windows (usually done when exiting the application)
    pub fn save(&mut self, filename: &str, view_plugins: &mut ViewPlugins) {
        println!("window len {}", self.windows.len());
        // TODO: This only supports one window for now
        if self.windows.len() == 1 {
            // TODO: Proper error handling here
            println!("save layout");
            self.windows[0].save_layout(filename, view_plugins).unwrap();
        }
    }

    /// Load the state of all the views from a previous run
    pub fn load(&mut self, filename: &str, view_plugins: &mut ViewPlugins) {
        // TODO: This only supports one window for now
        if self.windows.len() == 1 {
            // TODO: Proper error handling here (loading is ok to fail though)
            let _ = self.windows[0].load_layout(filename, view_plugins);
        }
    }

    fn setup_imgui_key_mappings() {
        Imgui::map_key(ImguiKey::Tab as usize, Key::Tab as usize);
        Imgui::map_key(ImguiKey::LeftArrow as usize, Key::Left as usize);
        Imgui::map_key(ImguiKey::RightArrow as usize, Key::Right as usize);
        Imgui::map_key(ImguiKey::DownArrow as usize, Key::Down as usize);
        Imgui::map_key(ImguiKey::UpArrow as usize, Key::Up as usize);
        Imgui::map_key(ImguiKey::PageUp as usize, Key::PageUp as usize);
        Imgui::map_key(ImguiKey::PageDown as usize, Key::PageDown as usize);
        Imgui::map_key(ImguiKey::Home as usize, Key::Home as usize);
        Imgui::map_key(ImguiKey::End as usize, Key::End as usize);
        Imgui::map_key(ImguiKey::Delete as usize, Key::Delete as usize);
        Imgui::map_key(ImguiKey::Backspace as usize, Key::Backspace as usize);
        Imgui::map_key(ImguiKey::Enter as usize, Key::Enter as usize);
        Imgui::map_key(ImguiKey::Escape as usize, Key::Escape as usize);
        Imgui::map_key(ImguiKey::A as usize, Key::A as usize);
        Imgui::map_key(ImguiKey::C as usize, Key::C as usize);
        Imgui::map_key(ImguiKey::V as usize, Key::V as usize);
        Imgui::map_key(ImguiKey::X as usize, Key::X as usize);
        Imgui::map_key(ImguiKey::Y as usize, Key::Y as usize);
        Imgui::map_key(ImguiKey::Z as usize, Key::Z as usize);
    }
}

impl Window {
    fn is_inside(v: (f32, f32), pos: PDVec2, size: (f32, f32)) -> bool {
        let x0 = pos.x;
        let y0 = pos.y;
        let x1 = pos.x + size.0;
        let y1 = pos.y + size.1;

        if (v.0 >= x0 && v.0 < x1) && (v.1 >= y0 && v.1 < y1) {
            true
        } else {
            false
        }
    }

    fn update_view(ws: &mut Workspace,
                   instance: &mut ViewInstance,
                   session: &mut Session,
                   show_context_menu: bool,
                   mouse: (f32, f32),
                   overlay: &Option<(DockHandle, Rect)>)
        -> WindowState {
            let ui = &instance.ui;

            if let Some(ref root) = ws.root_area {
                if let Some(ref container) =
                    root.get_container_by_dock_handle(DockHandle(instance.handle.0)) {
                        if container.docks[container.active_dock].handle.0 != instance.handle.0 {
                            return WindowState {
                                showed_popup: 0,
                                should_close: false,
                            };
                        }
                    }
            }

            if let Some(rect) = ws.get_rect_by_handle(DockHandle(instance.handle.0)) {
                Imgui::set_window_pos(rect.x, rect.y);
                Imgui::set_window_size(rect.width, rect.height);
            }

            let open = Imgui::begin_window(&instance.name, true);

            let mut has_tabs = false;
            if let Some(ref mut root) = ws.root_area {
                if let Some(ref mut container) =
                    root.get_container_by_dock_handle_mut(DockHandle(instance.handle.0)) {
                        let tabs: Vec<String> =
                            container.docks.iter().map(|dock| dock.plugin_name.clone()).collect();
                        if tabs.len() > 1 {
                            has_tabs = true;
                            Imgui::begin_window_child("tabs", 20.0);
                            let mut borders = Vec::with_capacity(tabs.len());
                            for (i, t) in tabs.iter().enumerate() {
                                if Imgui::tab(t, i == container.active_dock, i == tabs.len() - 1) {
                                    container.active_dock = i;
                                }
                                borders.push(Imgui::tab_pos());
                            }
                            container.update_tab_borders(&borders);
                            Imgui::end_window_child();
                            Imgui::separator();
                            Imgui::begin_window_child("body", 0.0);
                        }
                    }
            }

            Imgui::init_state(ui.api);

            let pos = ui.get_window_pos();
            let size = ui.get_window_size();

            if Self::is_inside(mouse, pos, size) && show_context_menu {
                Imgui::mark_show_popup(ui.api, true);
            } else {
                Imgui::mark_show_popup(ui.api, false);
            }

            // Draw drag zone
            if let &Some((handle, rect)) = overlay {
                if handle.0 == instance.handle.0 {
                    Imgui::render_frame(rect.x, rect.y, rect.width, rect.height, OVERLAY_COLOR);
                }
            }

            // Make sure we move the cursor to the start of the stream here
            ReaderWrapper::reset_reader(&mut session.reader);

            unsafe {
                let plugin_funcs = instance.plugin_type.plugin_funcs as *mut CViewCallbacks;
                ((*plugin_funcs).update.unwrap())(instance.plugin_data,
                                                  ui.api as *mut c_void,
                                                  session.reader.api as *mut c_void,
                                                  session.get_current_writer().api as *mut c_void);
            }

            let has_shown_menu = Imgui::has_showed_popup(ui.api);

            if has_tabs {
                Imgui::end_window_child();
            }
            Imgui::end_window();

            WindowState {
                showed_popup: has_shown_menu,
                should_close: !open,
            }
        }

    pub fn remove_views(&mut self, view_plugins: &mut ViewPlugins, views: &Vec<ViewHandle>) {
        for view in views {
            view_plugins.destroy_instance(*view);
            if let Some(pos) = self.views.iter().position(|v| v == view) {
                self.views.swap_remove(pos);
            }
            self.ws.delete_dock_by_handle(DockHandle(view.0));
        }
    }

    fn update_mouse_state(&mut self, mouse_pos: (f32, f32)) {
        let mut next_state = None;
        let cursor;
        let mut should_save_ws_state = false;
        match self.mouse_state.state {
            State::Default => {
                if let Some(sizer) = self.ws.get_sizer_at_pos(mouse_pos) {
                    cursor = match sizer.2 {
                        Direction::Vertical => CursorStyle::ResizeLeftRight,
                        Direction::Horizontal => CursorStyle::ResizeUpDown,
                    };
                    if self.win.get_mouse_down(MouseButton::Left) {
                        self.mouse_state.prev_mouse = mouse_pos;
                        next_state = Some(State::DraggingSizer(sizer));
                    }
                } else if let Some(handle) = self.ws.get_dock_handle_with_header_at_pos(mouse_pos) {
                    cursor = CursorStyle::ClosedHand;
                    if self.win.get_mouse_down(MouseButton::Left) {
                        self.mouse_state.prev_mouse = mouse_pos;
                        next_state = Some(State::PreDraggingDock(handle));
                    }
                } else {
                    cursor = CursorStyle::Arrow;
                    if self.win.get_mouse_down(MouseButton::Left) {
                        next_state = Some(State::DraggingNothing);
                    }
                }
            }

            State::DraggingNothing => {
                cursor = CursorStyle::Arrow;
                if !self.win.get_mouse_down(MouseButton::Left) {
                    next_state = Some(State::Default);
                }
            }

            State::DraggingSizer(SizerPos(handle, index, direction, origin_ratio)) => {
                if self.win.get_mouse_down(MouseButton::Left) {
                    cursor = match direction {
                        Direction::Vertical => CursorStyle::ResizeLeftRight,
                        Direction::Horizontal => CursorStyle::ResizeUpDown,
                    };
                    let pm = self.mouse_state.prev_mouse;
                    let delta = (pm.0 - mouse_pos.0, pm.1 - mouse_pos.1);
                    self.ws.change_ratio(handle, index, origin_ratio, delta);
                } else {
                    next_state = Some(State::Default);
                    cursor = CursorStyle::Arrow;
                    should_save_ws_state = true;
                }
            }

            State::PreDraggingDock(handle) => {
                cursor = CursorStyle::Arrow;
                if self.win.get_mouse_down(MouseButton::Left) {
                    let pm = self.mouse_state.prev_mouse;
                    let delta = (pm.0 - mouse_pos.0, pm.1 - mouse_pos.1);
                    if delta.0.abs() >= 5.0 && delta.1.abs() >= 5.0 {
                        next_state = Some(State::DraggingDock(handle));
                    }
                } else {
                    next_state = Some(State::Default);
                }
            }

            State::DraggingDock(handle) => {
                let mut move_target = self.ws.get_item_target_at_pos(mouse_pos);

                if let Some(target_handle) = self.ws.get_dock_handle_at_pos(mouse_pos) {
                    if target_handle == handle {
                        move_target = None;
                    }
                }

                if self.win.get_mouse_down(MouseButton::Left) {
                    cursor = match move_target {
                        Some(_) => CursorStyle::OpenHand,
                        None => CursorStyle::ClosedHand,
                    };
                    if let Some((_, rect)) = move_target {
                        if let Some(dh) = self.ws.get_dock_handle_at_pos(mouse_pos) {
                            self.overlay = Some((dh, rect));
                        } else {
                            self.overlay = None;
                        }
                    } else {
                        self.overlay = None;
                    }
                } else {
                    if let Some((target, _)) = move_target {
                        if !self.ws.already_at_place(&target, handle) {
                            self.ws.move_dock(handle, target);
                            self.save_cur_workspace_state();
                        }
                    }
                    next_state = Some(State::Default);
                    cursor = CursorStyle::Arrow;
                    self.overlay = None;
                }
            }
        }

        self.win.set_cursor_style(cursor);
        if let Some(ns) = next_state {
            self.mouse_state.state = ns;
        }
        if should_save_ws_state {
            self.save_cur_workspace_state();
        }
    }

    fn update_key_state(&mut self) {
        Imgui::clear_keys();

        self.win.get_keys_pressed(KeyRepeat::Yes).map(|keys| {
            for k in keys {
                Imgui::set_key_down(k as usize);
            }
        });
    }

    fn show_unix_menus(window: &minifb::Window) -> Option<usize> {
        let _ui = Imgui::get_ui();
        let _menus = window.get_unix_menus().unwrap();
        // implement unix menus here
        None
    }

    fn is_menu_pressed(window: &mut minifb::Window) -> Option<usize> {
        if window.get_unix_menus().is_some() {
            return Self::show_unix_menus(&window);
        }

        window.is_menu_pressed()
    }

    fn has_source_code_view(&self) -> bool {
        // TODO: Use setting for this name
        for dock in self.ws.get_docks() {
            if dock.plugin_name == "Source Code View" {
                return true;
            }
        }

        false
    }

    fn open_source_file(&mut self, filename: &str, 
                        view_plugins: &mut ViewPlugins,
                        session: &mut Session) {
        // check if we already have a source view open and just post the message.
        if !self.has_source_code_view() {
            let mouse = self.win.get_mouse_pos(MouseMode::Clamp).unwrap_or((0.0, 0.0));
            // This is somewhat hacky to set a "correct" split view for
            self.context_menu_data = self.ws.get_dock_handle_at_pos(mouse)
                                         .map(|handle| (handle, mouse));
            self.split_view(&"Source Code View".to_owned(), view_plugins, Direction::Vertical);
        }

        let writer = session.get_current_writer();
        writer.event_begin(events::EVENT_SET_SOURCE_CODE_FILE as u16);
        writer.write_string("filename", filename);
        writer.event_end();
    }

    fn browse_source_file(&mut self,
                          view_plugins: &mut ViewPlugins,
                          session: &mut Session) {
        match nfd::dialog().open() {
            Ok(NfdResponse::Cancel) => return,
            Err(e) => println!("Failed to open file dialog {:?}", e),
            Ok(NfdResponse::Okay(file)) => self.open_source_file(&file, view_plugins, session),
            _ => (),
        }
    }

    fn update_menus(&mut self,
                    view_plugins: &mut ViewPlugins,
                    sessions: &mut Sessions,
                    backend_plugins: &mut BackendPlugins) {
        let current_session = sessions.get_current();

        let menu_id = match Self::is_menu_pressed(&mut self.win) {
            Some(id) => id,
            None => return,
        };

        match menu_id {
            MENU_DEBUG_STEP_IN => current_session.action_step(),
            MENU_DEBUG_STEP_OVER => current_session.action_step_over(),
            MENU_DEBUG_START => current_session.action_run(),
            MENU_FILE_OPEN_SOURCE => self.browse_source_file(view_plugins, current_session),
            MENU_FILE_START_NEW_BACKEND => {
                if let Some(backend) =
                    backend_plugins.create_instance(&"Amiga UAE Debugger".to_owned()) {
                        current_session.set_backend(Some(backend));

                        if let Some(menu) = backend_plugins.get_menu(backend, self.menu_id_offset) {
                            self.win.add_menu(&(*menu));
                            self.menu_id_offset += 1000;
                        }
                    }
            }
            _ => {
                current_session.send_menu_id(menu_id as u32, backend_plugins);
            }
        }
    }

    pub fn pre_update(&mut self) {
        let mouse = self.win.get_mouse_pos(MouseMode::Clamp).unwrap_or((0.0, 0.0));
        Imgui::set_mouse_pos(mouse);
        Imgui::set_mouse_state(0, self.win.get_mouse_down(MouseButton::Left));
        if let Some(scroll) = self.win.get_scroll_wheel() {
            Imgui::set_scroll(scroll.1 * 0.25);
        }
    }

    pub fn update(&mut self,
                  sessions: &mut Sessions,
                  view_plugins: &mut ViewPlugins,
                  backend_plugins: &mut BackendPlugins) {
        let mut views_to_delete = Vec::new();
        let mut has_shown_menu = 0u32;

        let mut win_size = self.win.get_size();
        win_size.1 -= self.statusbar.get_size() as usize;

        let mouse = self.win.get_mouse_pos(MouseMode::Clamp).unwrap_or((0.0, 0.0));

        if has_shown_menu == 0 {
            self.update_mouse_state(mouse);
        }

        self.win.update();
        self.ws.update_rect(Rect::new(0.0, 0.0, win_size.0 as f32, win_size.1 as f32));
        self.update_key_state();

        let show_context_menu = self.win.get_mouse_down(MouseButton::Right);
        if show_context_menu {
            self.context_menu_data = self.ws
                .get_dock_handle_at_pos(mouse)
                .map(|handle| (handle, mouse));
        }

        for view in &self.views {
            if let Some(ref mut v) = view_plugins.get_view(*view) {
                if let Some(ref mut s) = sessions.get_session(v.session_handle) {
                    let state = Self::update_view(&mut self.ws,
                                                  v,
                                                  s,
                                                  show_context_menu,
                                                  mouse,
                                                  &self.overlay);

                    if state.should_close {
                        views_to_delete.push(*view);
                    }
                    has_shown_menu |= state.showed_popup;
                }
            }
        }

        self.statusbar.update(self.win.get_size());

        self.update_menus(view_plugins, sessions, backend_plugins);

        if self.win.is_key_pressed(Key::Z, KeyRepeat::No) {
            self.undo_workspace_change(view_plugins);
        }

        if self.win.is_key_pressed(Key::X, KeyRepeat::No) {
            self.redo_workspace_change(view_plugins);
        }

        // TODO: Only do this on the correct session

        // if self.win.is_key_pressed(Key::Down, KeyRepeat::No) {
        // self.ws.dump_tree();
        // }
        //
        // if self.win.is_key_pressed(Key::Up, KeyRepeat::No) {
        // self.save_layout("data/layout_temp.json", view_plugins);
        // }
        //
        // if self.win.is_key_pressed(Key::Right, KeyRepeat::No) {
        // self.load_layout("data/layout_temp.json", view_plugins);
        // }
        //

        // test

        // if now plugin has showed a menu we do it here
        // TODO: Handle diffrent cases when attach menu on to plugin menu or not

        if has_shown_menu == 0 && show_context_menu {
            Self::show_popup(self, true, view_plugins);
        } else {
            Self::show_popup(self, false, view_plugins);
        }

        if !views_to_delete.is_empty() {
            Self::remove_views(self, view_plugins, &views_to_delete);
            self.save_cur_workspace_state();
        }
    }

    fn restore_workspace_state(&mut self, view_plugins: &mut ViewPlugins) {
        self.ws = Workspace::from_state(&self.ws_states[self.cur_state_index]);
        let win_size = self.win.get_size();
        self.ws.update_rect(Rect::new(0.0, 0.0, win_size.0 as f32, win_size.1 as f32));
        let docks = self.ws.get_docks();
        let views_to_delete: Vec<ViewHandle> = self.views
            .iter()
            .filter(|view| docks.iter().find(|dock| view.0 == dock.handle.0).is_none())
            .map(|view| view.clone())
            .collect();
        Self::remove_views(self, view_plugins, &views_to_delete);

        for dock in &docks {
            let mut new_view_handles: Vec<ViewHandle> = Vec::new();
            if !self.views.iter().find(|view| view.0 == dock.handle.0).is_some() {
                let ui = Imgui::create_ui_instance();
                if let Some(handle) = view_plugins.create_instance_with_handle(ui,
                                                                               &dock.plugin_name,
                                                                               &dock.plugin_data,
                                                                               SessionHandle(0),
                                                                               ViewHandle(dock.handle.0)) {
                    new_view_handles.push(handle);
                } else {
                    panic!("Could not restore view");
                }
            }
            self.views.extend(new_view_handles);
        }
    }

    fn undo_workspace_change(&mut self, view_plugins: &mut ViewPlugins) {
        if self.cur_state_index > 0 {
            self.cur_state_index -= 1;
            self.restore_workspace_state(view_plugins);
        }
    }

    fn redo_workspace_change(&mut self, view_plugins: &mut ViewPlugins) {
        if self.cur_state_index < self.ws_states.len() - 1 {
            self.cur_state_index += 1;
            self.restore_workspace_state(view_plugins);
        }
    }

    fn save_cur_workspace_state(&mut self) {
        let state = self.ws.save_state();
        self.save_workspace_state(state);
    }

    fn save_workspace_state(&mut self, state: String) {
        self.ws_states.drain(self.cur_state_index + 1..);
        if self.cur_state_index == WORKSPACE_UNDO_LIMIT - 1 {
            self.ws_states.pop_front();
            self.cur_state_index -= 1;
        }
        self.ws_states.push_back(state);
        self.cur_state_index += 1;
    }

    fn split_view(&mut self, name: &String, view_plugins: &mut ViewPlugins, direction: Direction) {
        let ui = Imgui::create_ui_instance();
        if let Some(handle) = view_plugins.create_instance(ui, name, SessionHandle(0)) {
            let new_dock = Dock::new(DockHandle(handle.0), name);
            if let Some((dock_handle, pos)) = self.context_menu_data {
                let position = self.ws
                    .get_rect_by_handle(dock_handle)
                    .map(|rect| {
                        let lower_rect = rect.split_by_direction(direction, &[0.5])[0];
                        return if lower_rect.point_is_inside(pos) {
                            0
                        } else {
                            1
                        };
                    })
                .unwrap_or(1);
                self.ws.create_dock_at(ItemTarget::SplitDock(dock_handle, direction, position),
                new_dock);
            } else {
                self.ws.initialize(new_dock);
            }

            self.save_cur_workspace_state();
            self.views.push(handle);
        }
    }

    fn tab_view(&mut self, name: &String, view_plugins: &mut ViewPlugins) {
        let ui = Imgui::create_ui_instance();
        if let Some(handle) = view_plugins.create_instance(ui, name, SessionHandle(0)) {
            let new_handle = DockHandle(handle.0);
            let dock = viewdock::Dock::new(new_handle, name);
            self.views.push(handle);

            let mut should_save_ws = false;
            if let Some((src_dock_handle, _)) = self.context_menu_data {
                if let Some(ref mut root) = self.ws.root_area {
                    if let Some(ref mut container) =
                        root.get_container_by_dock_handle_mut(src_dock_handle) {
                            container.append_dock(dock);
                            should_save_ws = true;
                        }
                }
            }
            if should_save_ws {
                self.save_cur_workspace_state();
            }
        }
    }

    fn show_popup_menu_no_splits(&mut self,
                                 plugin_names: &Vec<String>,
                                 view_plugins: &mut ViewPlugins) {
        let ui = Imgui::get_ui();

        if ui.begin_menu("New View", true) {
            for name in plugin_names {
                if ui.menu_item(name, false, true) {
                    Self::split_view(self, &name, view_plugins, Direction::Horizontal);
                }
            }
            ui.end_menu();
        }
    }

    fn show_popup_change_view(&mut self,
                              plugin_names: &Vec<String>,
                              view_plugins: &mut ViewPlugins) {
        let ui = Imgui::get_ui();

        if ui.begin_menu("Change View", true) {
            for name in plugin_names {
                if ui.menu_item(name, false, true) {
                    if let Some((dock_handle, _)) = self.context_menu_data {
                        view_plugins.destroy_instance(ViewHandle(dock_handle.0));
                        view_plugins.create_instance_with_handle(Imgui::create_ui_instance(),
                        &name,
                        &None,
                        SessionHandle(0),
                        ViewHandle(dock_handle.0));
                    }
                }
            }
            ui.end_menu();
        }
    }

    fn show_popup_regular(&mut self, plugin_names: &Vec<String>, view_plugins: &mut ViewPlugins) {
        let ui = Imgui::get_ui();

        self.show_popup_change_view(plugin_names, view_plugins);

        if ui.begin_menu("Split Horizontally", true) {
            for name in plugin_names {
                if ui.menu_item(name, false, true) {
                    Self::split_view(self, &name, view_plugins, Direction::Horizontal);
                }
            }
            ui.end_menu();
        }

        if ui.begin_menu("Split Vertically", true) {
            for name in plugin_names {
                if ui.menu_item(name, false, true) {
                    Self::split_view(self, &name, view_plugins, Direction::Vertical);
                }
            }
            ui.end_menu();
        }

        if ui.begin_menu("Tab", true) {
            for name in plugin_names {
                if ui.menu_item(name, false, true) {
                    Self::tab_view(self, &name, view_plugins);
                }
            }
            ui.end_menu();
        }
    }

    fn show_popup(&mut self, show: bool, view_plugins: &mut ViewPlugins) {
        let ui = Imgui::get_ui();

        if show {
            ui.open_popup("plugins");
        }

        if ui.begin_popup("plugins") {
            let plugin_names = view_plugins.get_plugin_names();

            if self.ws.root_area.is_none() {
                self.show_popup_menu_no_splits(&plugin_names, view_plugins);
            } else {
                self.show_popup_regular(&plugin_names, view_plugins);
            }

            ui.end_popup();
        }
    }

    pub fn save_layout(&mut self,
                       filename: &str,
                       _view_plugins: &mut ViewPlugins)
        -> io::Result<()> {
            let mut file = try!(File::create(filename));
            let state = self.ws.save_state();
            println!("writing state to disk");
            file.write_all(state.as_str().as_bytes())
                // for split in &mut self.ws.splits {
                // let iter = split.left_docks.docks.iter_mut().chain(split.right_docks.docks.iter_mut());
                //
                // for dock in iter {
                // if let Some(ref plugin) = view_plugins.get_view(ViewHandle(dock.handle.0)) {
                // let (plugin_name, data) = plugin.get_plugin_data();
                // dock.plugin_name = plugin_name;
                // dock.plugin_data = data;
                // } else {
                // println!("Unable to find plugin for {:?} - this should never happen", dock);
                // }
                // }
                // }
                //
                // let _ = self.ws.save(filename);
                //
        }

    pub fn load_layout(&mut self,
                       filename: &str,
                       view_plugins: &mut ViewPlugins)
        -> io::Result<()> {
            let mut data = "".to_owned();

            let mut file = try!(File::open(filename));
            try!(file.read_to_string(&mut data));

            self.ws = Workspace::from_state(&data);

            let docks = self.ws.get_docks();

            // TODO: Move this code to seprate file and make it generic (copy'n'paste currently)
            for dock in &docks {
                let mut new_view_handles: Vec<ViewHandle> = Vec::new();
                if !self.views.iter().find(|view| view.0 == dock.handle.0).is_some() {
                    let ui = Imgui::create_ui_instance();
                    if let Some(handle) = view_plugins.create_instance_with_handle(ui,
                                                                                   &dock.plugin_name,
                                                                                   &dock.plugin_data,
                                                                                   SessionHandle(0),
                                                                                   ViewHandle(dock.handle.0)) {
                        new_view_handles.push(handle);
                    } else {
                        panic!("Could not restore view");
                    }
                }
                self.views.extend(new_view_handles);
            }

            Ok(())
        }
}
