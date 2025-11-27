use std::num::NonZeroU32;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_output, delegate_registry, delegate_seat, delegate_xdg_shell,
    delegate_xdg_window,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{Capability, SeatHandler, SeatState},
    shell::{
        xdg::{
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
            XdgShell,
        },
        WaylandSurface,
    },
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_seat, wl_surface},
    Connection, Proxy, QueueHandle,
};
use raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle,
    WaylandWindowHandle,
};
use glutin::{
    config::ConfigTemplateBuilder,
    context::{ContextAttributesBuilder, PossiblyCurrentContext},
    surface::{Surface, SurfaceAttributesBuilder, WindowSurface},
};
use glow::{Context, HasContext, COLOR_BUFFER_BIT, RENDERER, VERSION};

fn main() {
    env_logger::init();

    let conn = Connection::connect_to_env().unwrap();
    let (globals, mut event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();

    // Initialize xdg_shell handlers
    let compositor_state =
        CompositorState::bind(&globals, &qh).expect("wl_compositor not available");
    let xdg_shell_state = XdgShell::bind(&globals, &qh).expect("xdg shell not available");

    let surface = compositor_state.create_surface(&qh);
    // Create the window
    let window = xdg_shell_state.create_window(surface, WindowDecorations::ServerDefault, &qh);
    window.set_title("glow wayland window");
    // GitHub does not let projects use the `org.github` domain but the `io.github` domain is fine.
    window.set_app_id("io.github.smithay.client-toolkit.GlowExample");
    window.set_min_size(Some((256, 256)));
    window.commit();

    let mut glow_app = GlowApp {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),

        exit: false,
        width: 256,
        height: 256,
        window,
        conn,

        gl_display_context: None,
        gl_surface: None,
        gl_context: None,
    };

    // We don't draw immediately, the configure will notify us when to first draw.
    loop {
        event_queue.blocking_dispatch(&mut glow_app).unwrap();

        if glow_app.exit {
            println!("exiting example");
            break;
        }
    }

    // Clean up in the proper order
    drop(glow_app.gl_context);
    drop(glow_app.gl_surface);
    drop(glow_app.gl_display_context);
    drop(glow_app.window);
}

struct GlowApp {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,

    exit: bool,
    width: u32,
    height: u32,
    window: Window,
    conn: Connection,

    gl_display_context: Option<PossiblyCurrentContext>,
    gl_surface: Option<Surface<WindowSurface>>,
    gl_context: Option<Context>,
}

// Implement raw-window-handle traits for our window
struct WaylandWindow {
    display: *mut std::ffi::c_void,
    surface: *mut std::ffi::c_void,
}

impl HasDisplayHandle for WaylandWindow {
    fn display_handle(&self) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        let handle = WaylandDisplayHandle::new(std::ptr::NonNull::new(self.display).unwrap());
        Ok(unsafe { raw_window_handle::DisplayHandle::borrow_raw(RawDisplayHandle::Wayland(handle)) })
    }
}

impl HasWindowHandle for WaylandWindow {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let handle = WaylandWindowHandle::new(std::ptr::NonNull::new(self.surface).unwrap());
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(RawWindowHandle::Wayland(handle)) })
    }
}

impl GlowApp {
    fn init_gl(&mut self) {
        use glutin::prelude::*;

        // Create a window handle for glutin
        let wayland_window = WaylandWindow {
            display: self.conn.backend().display_ptr() as *mut _,
            surface: self.window.wl_surface().id().as_ptr() as *mut _,
        };

        // Create glutin display
        let gl_display = unsafe {
            glutin::display::Display::new(
                wayland_window.display_handle().unwrap().as_raw(),
                glutin::display::DisplayApiPreference::Egl,
            )
            .expect("Failed to create GL display")
        };

        // Configure the GL context
        let config_template = ConfigTemplateBuilder::new()
            .with_alpha_size(8)
            .with_transparency(true)
            .build();

        let config = unsafe {
            gl_display
                .find_configs(config_template)
                .expect("Failed to find GL configs")
                .next()
                .expect("No GL config found")
        };

        println!("Using GL config: {:?}", config);

        // Create GL context
        let context_attributes = ContextAttributesBuilder::new()
            .build(Some(wayland_window.window_handle().unwrap().as_raw()));

        let context = unsafe {
            gl_display
                .create_context(&config, &context_attributes)
                .expect("Failed to create GL context")
        };

        // Create GL surface
        let surface_attributes = SurfaceAttributesBuilder::<WindowSurface>::new()
            .build(
                wayland_window.window_handle().unwrap().as_raw(),
                NonZeroU32::new(self.width).unwrap(),
                NonZeroU32::new(self.height).unwrap(),
            );

        let surface = unsafe {
            gl_display
                .create_window_surface(&config, &surface_attributes)
                .expect("Failed to create GL surface")
        };

        // Make context current
        let context = context
            .make_current(&surface)
            .expect("Failed to make GL context current");

        // Create glow context
        let gl = unsafe {
            Context::from_loader_function(|s| {
                gl_display.get_proc_address(&std::ffi::CString::new(s).unwrap())
            })
        };

        println!("OpenGL version: {}", unsafe { gl.get_parameter_string(VERSION) });
        println!("OpenGL renderer: {}", unsafe { gl.get_parameter_string(RENDERER) });

        self.gl_display_context = Some(context);
        self.gl_surface = Some(surface);
        self.gl_context = Some(gl);
    }

    fn draw(&self) {
        use glutin::prelude::*;

        let gl = self.gl_context.as_ref().unwrap();
        let surface = self.gl_surface.as_ref().unwrap();
        let context = self.gl_display_context.as_ref().unwrap();

        // Clear the screen with a blue color
        unsafe {
            gl.clear_color(0.0, 0.0, 1.0, 1.0);
            gl.clear(COLOR_BUFFER_BIT);
        }

        // Swap buffers
        surface.swap_buffers(context)
            .expect("Failed to swap buffers");
        
        // Commit the surface to display the changes
        self.window.wl_surface().commit();
    }

    fn resize(&mut self, width: u32, height: u32) {
        use glutin::prelude::*;

        self.width = width;
        self.height = height;

        if let (Some(surface), Some(context)) = (&self.gl_surface, &self.gl_display_context) {
            surface.resize(
                context,
                NonZeroU32::new(width).unwrap(),
                NonZeroU32::new(height).unwrap(),
            );
            
            if let Some(gl) = &self.gl_context {
                unsafe {
                    gl.viewport(0, 0, width as i32, height as i32);
                }
            }
        }
    }
}

impl CompositorHandler for GlowApp {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
        // Not needed for this example.
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
        // Not needed for this example.
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
        // Not needed for this example.
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
        // Not needed for this example.
    }
}

impl OutputHandler for GlowApp {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl WindowHandler for GlowApp {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _window: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        let (new_width, new_height) = configure.new_size;
        let width = new_width.map_or(256, |v| v.get());
        let height = new_height.map_or(256, |v| v.get());

        // Initialize GL on first configure
        if self.gl_context.is_none() {
            self.width = width;
            self.height = height;
            self.init_gl();
            self.draw();
        } else if self.width != width || self.height != height {
            // Only resize and redraw if dimensions actually changed
            self.resize(width, height);
            self.draw();
        }
    }
}

impl SeatHandler for GlowApp {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        _capability: Capability,
    ) {
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        _capability: Capability,
    ) {
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

delegate_compositor!(GlowApp);
delegate_output!(GlowApp);
delegate_seat!(GlowApp);
delegate_xdg_shell!(GlowApp);
delegate_xdg_window!(GlowApp);
delegate_registry!(GlowApp);

impl ProvidesRegistryState for GlowApp {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}
