## GUI guide

ProDBG uses "immediate mode gui" approach to create UI. This means that ProDBG calls a set of methods to describe current gui state. Unlike traditional approach, ProDBG does not have initialization phase or add/remove methods for GUI objects. Instead, it just describes what should be on the screen and underlying library decides for itself what should be kept in cache, what should be created and what should be deleted. Developer may think of this approach as of mapping plugin internal structures into UI method calls.
 
## First example

Describe simple text and button here. Show example of button interaction.

## Layout model

Does ImGUI has one?

## Menus

### Usual menus

Do not use usual menus in plugin.

### Context menus and popups

Context menus are usual menus drawn in popup.
You are responsible for detecting when to show popup. See `Detecting mouse events` section for help. Once you decide a context menu should needs to be shown, do the next:

* mark popup to be shown with next code:
```
ui.open_popup("popup_id");
```
This should be done once when popup needs to be opened.

* draw popup with next code:
```
if ui.begin_popup("plugins") {
    // render menu here
    ui.end_popup(); // should only be called if ui.begin_popup() returned true
}
```
ui.begin_popup returns current popup state. By default every popup is hidden. When `ui.open_popup` is called, popup is marked shown. Popup state is managed by UI itself and popup is usually closed when user user clicks somewhere else.
TODO: describe how to close popup programmatically.

* menu is drawn using next functions:
```
if ui.begin_menu("Menu text", true) {
    if ui.begin_menu("Submenu text", true) {
        if ui.menu_item(name, false, true) {
            // Process item click
        }
        ui.end_menu();
    }
    ui.end_menu();
}
```
Each `begin_menu` starts new menu item with sub-menu. It returns `true` if item is hovered. It requires `ui.end_menu` if it returns `true`.
Each `menu_item` starts new "end-point" menu item. It returns `true` if item was clicked this frame.
TODO: describe other parameters.
Containing popup will be automatically closed if any menu item was clicked.

## Detecting mouse events

TODO: Describe is_mouse_hovered, etc.