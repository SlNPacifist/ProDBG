### Project structure

ProDBG is an application to debug executable files. Most of the crates mentioned here are developed especially for ProDBG. Right now ProDBG contains parts written both in C++ and Rust. ProDBG is in process of switching from C++ to Rust.

## Input and output

As every app, ProDBG needs to get user's input and draw its content in some sort of window, since it is not a console application.
Windows, os-specific menus and mouse cursor are drawn by [minifb](https://crates.io/crates/minifb) crate. Input is also provided by this crate.
Drawing inside of windows is done by [dear ImGui](https://github.com/ocornut/imgui).

## Plugins and backends

ProDBG is designed to be modular. Every module is a standalone compiled library. Module can be either backend (i.e. it will provide debugging functions to other modules) or plugin (i.e. it will provide user interface for some tasks).

## Entry point

ProDBG execution starts with the main crate located at `/src/prodbg/main`. If you look into `/src/prodbg/main/main.rs`, you will see that it includes all needed external parts of ProDBG and code for handling windows. At the start, it loads plugins from `t2-output` folder, creates new window and starts infinite loop to update everything (reload plugins if needed, redraw window).

## Adding new plugin as a part of ProDBG

All plugins keep their code in /src/plugins folder. Create new folder `my_plugin` there. After that, add new section in `/units.plugins.lua`. Section should look like this:
```
RustSharedLibrary {
	Name = "my_plugin",
	CargoConfig = "src/plugins/my_plugin/Cargo.toml",
	Sources = {
		get_rs_src("src/plugins/my_plugin"),
		get_rs_src("api/rust/prodbg"),
	}
}
```

And in the end of file add instruction to build that plugin by default:
```
Default "my_plugin"
```

Here we add entry to build our new plugin. Since `tundra` will only rebuild plugin if anything related changes, plugin author have to specify all dependencies carefully.
In this example we specify that our plugin should be rebuilt if any of its files change or anything in ProDBG API changes.

Initialize new crate in plugin folder:
```
cargo init
```

In your `cargo.toml` file change library type to `dylib`:
```
[lib]
name = "my_plugin"
crate-type = ["dylib"]
```

and add external crate for rust API:
```
[dependencies]
prodbg_api = { path = "../../../api/rust/prodbg" }
```

Now you can use Rust API in your `lib.rs`:

```
#[macro_use]
extern crate prodbg_api;

use prodbg_api::{View, Ui, Reader, Writer, PluginHandler, Service, CViewCallbacks};

struct MyPlugin {}

impl View for MyPlugin {
    fn new(ui: &Ui, _: &Service) -> Self {
        MyPlugin {}
    }

    fn update(&mut self, ui: &mut Ui, reader: &mut Reader, writer: &mut Writer) {
        ui.text("My plugin loaded")
    }
}

#[no_mangle]
pub fn init_plugin(plugin_handler: &mut PluginHandler) {
    define_view_plugin!(PLUGIN, b"My plugin\0", MyPlugin);
    plugin_handler.register_view(&PLUGIN);
}
```

Plugin is registered within `init_plugin` function. Usually plugin is just a structure implementing `prodgb_api::View` trait. In this case we can use `define_view_plugin!` macro. `prodgb_api::View::update` will be called on every redraw and will supply objects to interact with ProDBG:

* `Ui` is a ProDBG interface for drawing, mostly a direct `dear ImGui` mapping. More on the interface in `/api/rust/src/ui.rs`
* `Reader` and `Writer` are interfce for reading and writing messages. More on them in `/api/rust/src/read_write.rs`

