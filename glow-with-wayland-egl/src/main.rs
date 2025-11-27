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
use wayland_egl::WlEglSurface;
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

        egl_instance: None,
        wl_egl_surface: None,
        egl_display: None,
        egl_context: None,
        egl_surface: None,
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
    // EGL surface, context and display are Copy types and don't need explicit drops
    drop(glow_app.wl_egl_surface);
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

    egl_instance: Option<khronos_egl::Instance<khronos_egl::Static>>,
    wl_egl_surface: Option<WlEglSurface>,
    egl_display: Option<khronos_egl::Display>,
    egl_context: Option<khronos_egl::Context>,
    egl_surface: Option<khronos_egl::Surface>,
    gl_context: Option<Context>,
}

impl GlowApp {
    fn init_gl(&mut self) {
        // Get the native display
        let display_ptr = self.conn.backend().display_ptr();
        
        // Initialize EGL
        let egl = khronos_egl::Instance::new(khronos_egl::Static);
        let display = unsafe {
            egl.get_display(display_ptr as *mut std::ffi::c_void)
                .expect("Failed to get EGL display")
        };
        
        egl.initialize(display).expect("Failed to initialize EGL");

        let version = egl.query_string(Some(display), khronos_egl::VERSION)
            .expect("Failed to query EGL version");
        println!("EGL version: {:?}", version);

        // Choose an EGL config
        let attributes = [
            khronos_egl::RED_SIZE, 8,
            khronos_egl::GREEN_SIZE, 8,
            khronos_egl::BLUE_SIZE, 8,
            khronos_egl::ALPHA_SIZE, 8,
            khronos_egl::SURFACE_TYPE, khronos_egl::WINDOW_BIT,
            khronos_egl::RENDERABLE_TYPE, khronos_egl::OPENGL_ES2_BIT,
            khronos_egl::NONE,
        ];

        let config = egl
            .choose_first_config(display, &attributes)
            .expect("Failed to choose EGL config")
            .expect("No EGL config found");

        // Bind OpenGL ES API
        egl.bind_api(khronos_egl::OPENGL_ES_API).expect("Failed to bind OpenGL ES API");

        // Create EGL context
        let context_attributes = [
            khronos_egl::CONTEXT_MAJOR_VERSION, 2,
            khronos_egl::CONTEXT_MINOR_VERSION, 0,
            khronos_egl::NONE,
        ];

        let context = egl
            .create_context(display, config, None, &context_attributes)
            .expect("Failed to create EGL context");

        // Create the wayland EGL surface
        let wl_egl_surface = WlEglSurface::new(
            self.window.wl_surface().id(),
            self.width as i32,
            self.height as i32,
        )
        .expect("Failed to create WlEglSurface");

        // Create EGL window surface
        let egl_surface = unsafe {
            egl.create_window_surface(
                display,
                config,
                wl_egl_surface.ptr() as *mut std::ffi::c_void,
                None,
            )
            .expect("Failed to create EGL surface")
        };

        // Make the context current
        egl.make_current(display, Some(egl_surface), Some(egl_surface), Some(context))
            .expect("Failed to make EGL context current");

        // Set swap interval to 0 for non-blocking rendering
        egl.swap_interval(display, 0).ok();

        // Create glow context
        let gl = unsafe {
            Context::from_loader_function(|s| {
                egl.get_proc_address(s).unwrap() as *const _
            })
        };

        println!("OpenGL version: {}", unsafe { gl.get_parameter_string(VERSION) });
        println!("OpenGL renderer: {}", unsafe { gl.get_parameter_string(RENDERER) });

        self.egl_instance = Some(egl);
        self.wl_egl_surface = Some(wl_egl_surface);
        self.egl_display = Some(display);
        self.egl_context = Some(context);
        self.egl_surface = Some(egl_surface);
        self.gl_context = Some(gl);
    }

    fn draw(&self) {
        let gl = self.gl_context.as_ref().unwrap();
        let egl = self.egl_instance.as_ref().unwrap();
        let display = self.egl_display.unwrap();
        let egl_surface = self.egl_surface.unwrap();

        // Clear the screen with a blue color
        unsafe {
            gl.clear_color(0.0, 0.0, 1.0, 1.0);
            gl.clear(COLOR_BUFFER_BIT);
        }

        // Swap buffers (non-blocking due to swap interval = 0)
        egl.swap_buffers(display, egl_surface)
            .expect("Failed to swap buffers");
        
        // Commit the surface to display the changes
        self.window.wl_surface().commit();
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;

        if let Some(wl_egl_surface) = &self.wl_egl_surface {
            wl_egl_surface.resize(width as i32, height as i32, 0, 0);
            
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
