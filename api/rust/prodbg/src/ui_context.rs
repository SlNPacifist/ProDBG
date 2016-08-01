use ui::{Ui, ImGuiStyleVar};
use ui_ffi::PDVec2;

pub struct UiContext<'a> {
    ui: &'a mut Ui,
    handlers: Vec<Box<ContextSetter>>
}

impl<'a> UiContext<'a> {
    pub fn new(ui: &mut Ui) -> UiContext {
        UiContext {
            ui: ui,
            handlers: Vec::new(),
        }
    }

    pub fn style(&'a mut self, style: Style) -> &'a mut UiContext {
        self.handlers.push(Box::new(style));
        self
    }

    pub fn item_width(&'a mut self, width: f32) -> &'a mut UiContext {
        self.handlers.push(Box::new(ItemWidth(width)));
        self
    }

    pub fn exec(&mut self, func: &mut FnMut(&mut Ui)) {
        for handler in self.handlers.iter() {
            handler.push(self.ui);
        }
        func(self.ui);
        for handler in self.handlers.iter().rev() {
            handler.pop(self.ui);
        }
    }
}

pub trait ContextSetter {
    fn push(&self, &mut Ui);
    fn pop(&self, &mut Ui);
}

// Style

pub enum Style {
    Single(ImGuiStyleVar, f32),
    Vec2(ImGuiStyleVar, PDVec2),
}

impl Style {
    pub fn single(var: ImGuiStyleVar, val: f32) -> Style {
        Style::Single(var, val)
    }
    pub fn vec2(var: ImGuiStyleVar, x: f32, y: f32) -> Style {
        Style::Vec2(var, PDVec2{x: x, y: y})
    }
}

impl ContextSetter for Style {
    fn push(&self, ui: &mut Ui) {
        match *self {
            Style::Single(ref index, ref val) => ui.push_style_var(index.clone(), val.clone()),
            Style::Vec2(ref index, ref val) => ui.push_style_var_vec(index.clone(), val.clone()),
        }
    }

    fn pop(&self, ui: &mut Ui) {
        ui.pop_style_var(1);
    }
}

// Width

pub struct ItemWidth(f32);

impl ContextSetter for ItemWidth {
    fn push(&self, ui: &mut Ui) {
        ui.push_item_width(self.0);
    }

    fn pop(&self, ui: &mut Ui) {
        ui.pop_item_width();
    }
}