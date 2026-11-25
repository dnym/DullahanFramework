//! Linux zero-dependency platform backend:
//! - Windowed mode: `libX11.so.6` + `libGL.so.1` (GLX 1.4 -> OpenGL 3.3 Core)
//! - Headless mode: `libEGL.so.1` (surfaceless / pbuffer OpenGL 3.3 Core + offscreen FBO)

use crate::dullahan::math::Vec2;
use crate::dullahan::platform::{
    GL_CLAMP_TO_EDGE, GL_COLOR_ATTACHMENT0, GL_DEPTH_ATTACHMENT, GL_DEPTH_COMPONENT24,
    GL_FRAMEBUFFER, GL_FRAMEBUFFER_COMPLETE, GL_NEAREST, GL_RENDERBUFFER, GL_RGBA, GL_RGBA8,
    GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_TEXTURE_MIN_FILTER, GL_TEXTURE_WRAP_S,
    GL_TEXTURE_WRAP_T, GL_UNSIGNED_BYTE, GLuint, GlFns, InputState, Key, MouseButton,
};
use std::ffi::{CString, c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::ptr;

// ---------------------------------------------------------------------------
// Dynamic loading via libc
// ---------------------------------------------------------------------------

const RTLD_NOW: c_int = 2;
const RTLD_GLOBAL: c_int = 0x100;

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
}

struct DynLib(*mut c_void);

impl DynLib {
    fn open(names: &[&str]) -> Result<Self, String> {
        for name in names {
            let c = CString::new(*name).unwrap();
            let h = unsafe { dlopen(c.as_ptr(), RTLD_NOW | RTLD_GLOBAL) };
            if !h.is_null() {
                return Ok(Self(h));
            }
        }
        Err(format!("failed to dlopen any of {:?}", names))
    }

    fn sym_raw(&self, name: &str) -> *const c_void {
        let c = CString::new(name).unwrap();
        unsafe { dlsym(self.0, c.as_ptr()) as *const c_void }
    }

    unsafe fn sym<T>(&self, name: &str) -> Result<T, String> {
        let p = self.sym_raw(name);
        if p.is_null() {
            Err(format!("dlsym failed: {name}"))
        } else {
            Ok(unsafe { std::mem::transmute_copy(&p) })
        }
    }
}

impl Drop for DynLib {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                dlclose(self.0);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// X11 + GLX types and constants
// ---------------------------------------------------------------------------

type XDisplay = c_void;
type XWindow = c_ulong;
type XColormap = c_ulong;
type XAtom = c_ulong;
type GLXFBConfig = *mut c_void;
type GLXContext = *mut c_void;

#[repr(C)]
struct XVisualInfo {
    visual: *mut c_void,
    visualid: c_ulong,
    screen: c_int,
    depth: c_int,
    class: c_int,
    red_mask: c_ulong,
    green_mask: c_ulong,
    blue_mask: c_ulong,
    colormap_size: c_int,
    bits_per_rgb: c_int,
}

#[repr(C)]
struct XSetWindowAttributes {
    background_pixmap: c_ulong,
    background_pixel: c_ulong,
    border_pixmap: c_ulong,
    border_pixel: c_ulong,
    bit_gravity: c_int,
    win_gravity: c_int,
    backing_store: c_int,
    backing_planes: c_ulong,
    backing_pixel: c_ulong,
    save_under: c_int,
    event_mask: c_long,
    do_not_propagate_mask: c_long,
    override_redirect: c_int,
    colormap: XColormap,
    cursor: c_ulong,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct XKeyEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut XDisplay,
    window: XWindow,
    root: XWindow,
    subwindow: XWindow,
    time: c_ulong,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    state: c_uint,
    keycode: c_uint,
    same_screen: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct XButtonEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut XDisplay,
    window: XWindow,
    root: XWindow,
    subwindow: XWindow,
    time: c_ulong,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    state: c_uint,
    button: c_uint,
    same_screen: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct XMotionEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut XDisplay,
    window: XWindow,
    root: XWindow,
    subwindow: XWindow,
    time: c_ulong,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    state: c_uint,
    is_hint: c_char,
    same_screen: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct XConfigureEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut XDisplay,
    event: XWindow,
    window: XWindow,
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
    border_width: c_int,
    above: XWindow,
    override_redirect: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct XClientMessageEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut XDisplay,
    window: XWindow,
    message_type: XAtom,
    format: c_int,
    data: [c_long; 5],
}

#[repr(C)]
union XEvent {
    type_: c_int,
    xkey: XKeyEvent,
    xbutton: XButtonEvent,
    xmotion: XMotionEvent,
    xconfigure: XConfigureEvent,
    xclient: XClientMessageEvent,
    pad: [c_long; 24],
}

const KEY_PRESS: c_int = 2;
const KEY_RELEASE: c_int = 3;
const BUTTON_PRESS: c_int = 4;
const BUTTON_RELEASE: c_int = 5;
const MOTION_NOTIFY: c_int = 6;
const FOCUS_IN: c_int = 9;
const FOCUS_OUT: c_int = 10;
const CONFIGURE_NOTIFY: c_int = 22;
const CLIENT_MESSAGE: c_int = 33;

const KEY_PRESS_MASK: c_long = 1 << 0;
const KEY_RELEASE_MASK: c_long = 1 << 1;
const BUTTON_PRESS_MASK: c_long = 1 << 2;
const BUTTON_RELEASE_MASK: c_long = 1 << 3;
const POINTER_MOTION_MASK: c_long = 1 << 6;
const EXPOSURE_MASK: c_long = 1 << 15;
const STRUCTURE_NOTIFY_MASK: c_long = 1 << 17;
const FOCUS_CHANGE_MASK: c_long = 1 << 21;

const CW_BORDER_PIXEL: c_ulong = 1 << 3;
const CW_EVENT_MASK: c_ulong = 1 << 11;
const CW_COLORMAP: c_ulong = 1 << 13;
const INPUT_OUTPUT: c_uint = 1;

const GLX_X_RENDERABLE: c_int = 0x8012;
const GLX_DRAWABLE_TYPE: c_int = 0x8010;
const GLX_RENDER_TYPE: c_int = 0x8011;
const GLX_WINDOW_BIT: c_int = 0x0001;
const GLX_RGBA_BIT: c_int = 0x0001;
const GLX_DOUBLEBUFFER: c_int = 5;
const GLX_RED_SIZE: c_int = 8;
const GLX_GREEN_SIZE: c_int = 9;
const GLX_BLUE_SIZE: c_int = 10;
const GLX_ALPHA_SIZE: c_int = 11;
const GLX_DEPTH_SIZE: c_int = 12;
const GLX_CONTEXT_MAJOR_VERSION_ARB: c_int = 0x2091;
const GLX_CONTEXT_MINOR_VERSION_ARB: c_int = 0x2092;
const GLX_CONTEXT_PROFILE_MASK_ARB: c_int = 0x9126;
const GLX_CONTEXT_CORE_PROFILE_BIT_ARB: c_int = 0x0001;

#[allow(non_snake_case)]
struct X11Fns {
    XOpenDisplay: unsafe extern "C" fn(*const c_char) -> *mut XDisplay,
    XCloseDisplay: unsafe extern "C" fn(*mut XDisplay) -> c_int,
    XDefaultScreen: unsafe extern "C" fn(*mut XDisplay) -> c_int,
    XRootWindow: unsafe extern "C" fn(*mut XDisplay, c_int) -> XWindow,
    XCreateColormap: unsafe extern "C" fn(*mut XDisplay, XWindow, *mut c_void, c_int) -> XColormap,
    XFreeColormap: unsafe extern "C" fn(*mut XDisplay, XColormap) -> c_int,
    XCreateWindow: unsafe extern "C" fn(
        *mut XDisplay, XWindow, c_int, c_int, c_uint, c_uint, c_uint, c_int, c_uint,
        *mut c_void, c_ulong, *mut XSetWindowAttributes,
    ) -> XWindow,
    XDestroyWindow: unsafe extern "C" fn(*mut XDisplay, XWindow) -> c_int,
    XStoreName: unsafe extern "C" fn(*mut XDisplay, XWindow, *const c_char) -> c_int,
    XInternAtom: unsafe extern "C" fn(*mut XDisplay, *const c_char, c_int) -> XAtom,
    XSetWMProtocols: unsafe extern "C" fn(*mut XDisplay, XWindow, *mut XAtom, c_int) -> c_int,
    XMapWindow: unsafe extern "C" fn(*mut XDisplay, XWindow) -> c_int,
    XFlush: unsafe extern "C" fn(*mut XDisplay) -> c_int,
    XPending: unsafe extern "C" fn(*mut XDisplay) -> c_int,
    XNextEvent: unsafe extern "C" fn(*mut XDisplay, *mut XEvent) -> c_int,
    XLookupKeysym: unsafe extern "C" fn(*mut XKeyEvent, c_int) -> c_ulong,
    XFree: unsafe extern "C" fn(*mut c_void) -> c_int,
    XWarpPointer: unsafe extern "C" fn(*mut XDisplay, XWindow, XWindow, c_int, c_int, c_uint, c_uint, c_int, c_int) -> c_int,
    XCreatePixmap: unsafe extern "C" fn(*mut XDisplay, XWindow, c_uint, c_uint, c_uint) -> c_ulong,
    XFreePixmap: unsafe extern "C" fn(*mut XDisplay, c_ulong) -> c_int,
    XCreatePixmapCursor: unsafe extern "C" fn(*mut XDisplay, c_ulong, c_ulong, *mut XColor, *mut XColor, c_uint, c_uint) -> c_ulong,
    XDefineCursor: unsafe extern "C" fn(*mut XDisplay, XWindow, c_ulong) -> c_int,
    XUndefineCursor: unsafe extern "C" fn(*mut XDisplay, XWindow) -> c_int,
    XFreeCursor: unsafe extern "C" fn(*mut XDisplay, c_ulong) -> c_int,
}

#[repr(C)]
struct XColor {
    pixel: c_ulong,
    red: u16,
    green: u16,
    blue: u16,
    flags: c_char,
    pad: c_char,
}

#[allow(non_snake_case)]
struct GlxFns {
    glXChooseFBConfig: unsafe extern "C" fn(*mut XDisplay, c_int, *const c_int, *mut c_int) -> *mut GLXFBConfig,
    glXGetVisualFromFBConfig: unsafe extern "C" fn(*mut XDisplay, GLXFBConfig) -> *mut XVisualInfo,
    glXGetProcAddressARB: unsafe extern "C" fn(*const c_char) -> *const c_void,
    glXMakeCurrent: unsafe extern "C" fn(*mut XDisplay, XWindow, GLXContext) -> c_int,
    glXSwapBuffers: unsafe extern "C" fn(*mut XDisplay, XWindow),
    glXDestroyContext: unsafe extern "C" fn(*mut XDisplay, GLXContext),
}

// ---------------------------------------------------------------------------
// EGL types and constants (for --headless offscreen OpenGL 3.3 Core)
// ---------------------------------------------------------------------------

type EGLDisplay = *mut c_void;
type EGLConfig = *mut c_void;
type EGLContext = *mut c_void;
type EGLSurface = *mut c_void;
type EGLint = c_int;
type EGLenum = c_uint;
type EGLBoolean = c_uint;

const EGL_NONE: EGLint = 0x3038;
const EGL_RED_SIZE: EGLint = 0x3024;
const EGL_GREEN_SIZE: EGLint = 0x3023;
const EGL_BLUE_SIZE: EGLint = 0x3022;
const EGL_ALPHA_SIZE: EGLint = 0x3021;
const EGL_DEPTH_SIZE: EGLint = 0x3025;
const EGL_SURFACE_TYPE: EGLint = 0x3033;
const EGL_RENDERABLE_TYPE: EGLint = 0x3040;
const EGL_PBUFFER_BIT: EGLint = 0x0001;
const EGL_OPENGL_BIT: EGLint = 0x0008;
const EGL_WIDTH: EGLint = 0x3057;
const EGL_HEIGHT: EGLint = 0x3056;
const EGL_OPENGL_API: EGLenum = 0x30A2;
const EGL_CONTEXT_MAJOR_VERSION: EGLint = 0x3098;
const EGL_CONTEXT_MINOR_VERSION: EGLint = 0x30FB;
const EGL_CONTEXT_OPENGL_PROFILE_MASK: EGLint = 0x30FD;
const EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT: EGLint = 0x0000_0001;
const EGL_PLATFORM_SURFACELESS_MESA: EGLenum = 0x31DD;

#[allow(non_snake_case)]
struct EglFns {
    eglGetDisplay: unsafe extern "C" fn(*mut c_void) -> EGLDisplay,
    eglInitialize: unsafe extern "C" fn(EGLDisplay, *mut EGLint, *mut EGLint) -> EGLBoolean,
    eglTerminate: unsafe extern "C" fn(EGLDisplay) -> EGLBoolean,
    eglBindAPI: unsafe extern "C" fn(EGLenum) -> EGLBoolean,
    eglChooseConfig: unsafe extern "C" fn(EGLDisplay, *const EGLint, *mut EGLConfig, EGLint, *mut EGLint) -> EGLBoolean,
    eglCreatePbufferSurface: unsafe extern "C" fn(EGLDisplay, EGLConfig, *const EGLint) -> EGLSurface,
    eglDestroySurface: unsafe extern "C" fn(EGLDisplay, EGLSurface) -> EGLBoolean,
    eglCreateContext: unsafe extern "C" fn(EGLDisplay, EGLConfig, EGLContext, *const EGLint) -> EGLContext,
    eglDestroyContext: unsafe extern "C" fn(EGLDisplay, EGLContext) -> EGLBoolean,
    eglMakeCurrent: unsafe extern "C" fn(EGLDisplay, EGLSurface, EGLSurface, EGLContext) -> EGLBoolean,
    eglGetProcAddress: unsafe extern "C" fn(*const c_char) -> *const c_void,
}

// ---------------------------------------------------------------------------
// Backend storage
// ---------------------------------------------------------------------------

enum Backend {
    X11 {
        _lib_x11: DynLib,
        _lib_gl: DynLib,
        x11: X11Fns,
        glx: GlxFns,
        display: *mut XDisplay,
        window: XWindow,
        colormap: XColormap,
        context: GLXContext,
        wm_delete_window: XAtom,
        last_mouse: Option<Vec2>,
        cursor_locked: bool,
        blank_cursor: c_ulong,
    },
    HeadlessEgl {
        _lib_egl: DynLib,
        _lib_gl: DynLib,
        egl: EglFns,
        display: EGLDisplay,
        surface: EGLSurface,
        context: EGLContext,
        fbo: GLuint,
        color_tex: GLuint,
        depth_rb: GLuint,
    },
}

pub struct Platform {
    backend: Backend,
    pub gl: GlFns,
    pub width: u32,
    pub height: u32,
    pub headless: bool,
}

impl Platform {
    pub fn new(title: &str, width: u32, height: u32, headless: bool) -> Result<Self, String> {
        if headless {
            Self::new_headless_egl(width, height)
        } else {
            Self::new_x11(title, width, height)
        }
    }

    fn new_x11(title: &str, width: u32, height: u32) -> Result<Self, String> {
        let lib_x11 = DynLib::open(&["libX11.so.6", "libX11.so"])?;
        let lib_gl = DynLib::open(&["libGL.so.1", "libGL.so"])?;

        let x11 = unsafe {
            X11Fns {
                XOpenDisplay: lib_x11.sym("XOpenDisplay")?,
                XCloseDisplay: lib_x11.sym("XCloseDisplay")?,
                XDefaultScreen: lib_x11.sym("XDefaultScreen")?,
                XRootWindow: lib_x11.sym("XRootWindow")?,
                XCreateColormap: lib_x11.sym("XCreateColormap")?,
                XFreeColormap: lib_x11.sym("XFreeColormap")?,
                XCreateWindow: lib_x11.sym("XCreateWindow")?,
                XDestroyWindow: lib_x11.sym("XDestroyWindow")?,
                XStoreName: lib_x11.sym("XStoreName")?,
                XInternAtom: lib_x11.sym("XInternAtom")?,
                XSetWMProtocols: lib_x11.sym("XSetWMProtocols")?,
                XMapWindow: lib_x11.sym("XMapWindow")?,
                XFlush: lib_x11.sym("XFlush")?,
                XPending: lib_x11.sym("XPending")?,
                XNextEvent: lib_x11.sym("XNextEvent")?,
                XLookupKeysym: lib_x11.sym("XLookupKeysym")?,
                XFree: lib_x11.sym("XFree")?,
                XWarpPointer: lib_x11.sym("XWarpPointer")?,
                XCreatePixmap: lib_x11.sym("XCreatePixmap")?,
                XFreePixmap: lib_x11.sym("XFreePixmap")?,
                XCreatePixmapCursor: lib_x11.sym("XCreatePixmapCursor")?,
                XDefineCursor: lib_x11.sym("XDefineCursor")?,
                XUndefineCursor: lib_x11.sym("XUndefineCursor")?,
                XFreeCursor: lib_x11.sym("XFreeCursor")?,
            }
        };

        let glx = unsafe {
            GlxFns {
                glXChooseFBConfig: lib_gl.sym("glXChooseFBConfig")?,
                glXGetVisualFromFBConfig: lib_gl.sym("glXGetVisualFromFBConfig")?,
                glXGetProcAddressARB: lib_gl.sym("glXGetProcAddressARB")?,
                glXMakeCurrent: lib_gl.sym("glXMakeCurrent")?,
                glXSwapBuffers: lib_gl.sym("glXSwapBuffers")?,
                glXDestroyContext: lib_gl.sym("glXDestroyContext")?,
            }
        };

        let display = unsafe { (x11.XOpenDisplay)(ptr::null()) };
        if display.is_null() {
            return Err("XOpenDisplay failed (is DISPLAY set? use --headless for offscreen)".into());
        }

        let screen = unsafe { (x11.XDefaultScreen)(display) };
        let root = unsafe { (x11.XRootWindow)(display, screen) };

        let fb_attribs: [c_int; 19] = [
            GLX_X_RENDERABLE, 1,
            GLX_DRAWABLE_TYPE, GLX_WINDOW_BIT,
            GLX_RENDER_TYPE, GLX_RGBA_BIT,
            GLX_DOUBLEBUFFER, 1,
            GLX_RED_SIZE, 8,
            GLX_GREEN_SIZE, 8,
            GLX_BLUE_SIZE, 8,
            GLX_ALPHA_SIZE, 8,
            GLX_DEPTH_SIZE, 24,
            0,
        ];

        let mut fb_count: c_int = 0;
        let fb_configs = unsafe {
            (glx.glXChooseFBConfig)(display, screen, fb_attribs.as_ptr(), &mut fb_count)
        };
        if fb_configs.is_null() || fb_count == 0 {
            unsafe { (x11.XCloseDisplay)(display) };
            return Err("glXChooseFBConfig found no matching visual".into());
        }
        let fb_cfg = unsafe { *fb_configs };
        unsafe { (x11.XFree)(fb_configs as *mut c_void) };

        let vi = unsafe { (glx.glXGetVisualFromFBConfig)(display, fb_cfg) };
        if vi.is_null() {
            unsafe { (x11.XCloseDisplay)(display) };
            return Err("glXGetVisualFromFBConfig returned null".into());
        }

        let colormap = unsafe { (x11.XCreateColormap)(display, root, (*vi).visual, 0) };
        let mut swa: XSetWindowAttributes = unsafe { std::mem::zeroed() };
        swa.colormap = colormap;
        swa.border_pixel = 0;
        swa.event_mask = KEY_PRESS_MASK
            | KEY_RELEASE_MASK
            | BUTTON_PRESS_MASK
            | BUTTON_RELEASE_MASK
            | POINTER_MOTION_MASK
            | STRUCTURE_NOTIFY_MASK
            | FOCUS_CHANGE_MASK
            | EXPOSURE_MASK;

        let window = unsafe {
            (x11.XCreateWindow)(
                display,
                root,
                0,
                0,
                width,
                height,
                0,
                (*vi).depth,
                INPUT_OUTPUT,
                (*vi).visual,
                CW_BORDER_PIXEL | CW_COLORMAP | CW_EVENT_MASK,
                &mut swa,
            )
        };
        unsafe { (x11.XFree)(vi as *mut c_void) };

        let c_title = CString::new(title).unwrap_or_else(|_| CString::new("Dullahan").unwrap());
        unsafe { (x11.XStoreName)(display, window, c_title.as_ptr()) };

        let wm_del_name = CString::new("WM_DELETE_WINDOW").unwrap();
        let mut wm_delete_window = unsafe { (x11.XInternAtom)(display, wm_del_name.as_ptr(), 0) };
        unsafe { (x11.XSetWMProtocols)(display, window, &mut wm_delete_window, 1) };

        let create_ctx_name = CString::new("glXCreateContextAttribsARB").unwrap();
        let create_ctx_ptr = unsafe { (glx.glXGetProcAddressARB)(create_ctx_name.as_ptr()) };
        if create_ctx_ptr.is_null() {
            return Err("glXCreateContextAttribsARB unavailable".into());
        }
        type CreateCtxFn = unsafe extern "C" fn(*mut XDisplay, GLXFBConfig, GLXContext, c_int, *const c_int) -> GLXContext;
        let create_ctx: CreateCtxFn = unsafe { std::mem::transmute(create_ctx_ptr) };

        let ctx_attribs: [c_int; 7] = [
            GLX_CONTEXT_MAJOR_VERSION_ARB, 3,
            GLX_CONTEXT_MINOR_VERSION_ARB, 3,
            GLX_CONTEXT_PROFILE_MASK_ARB, GLX_CONTEXT_CORE_PROFILE_BIT_ARB,
            0,
        ];
        let context = unsafe { create_ctx(display, fb_cfg, ptr::null_mut(), 1, ctx_attribs.as_ptr()) };
        if context.is_null() {
            return Err("glXCreateContextAttribsARB failed for OpenGL 3.3 Core".into());
        }

        unsafe {
            (x11.XMapWindow)(display, window);
            (x11.XFlush)(display);
            (glx.glXMakeCurrent)(display, window, context);
        }

        let gl = GlFns::load(|name| {
            let c = CString::new(name).unwrap();
            let p = unsafe { (glx.glXGetProcAddressARB)(c.as_ptr()) };
            if !p.is_null() { p } else { lib_gl.sym_raw(name) }
        })?;

        // Create a 1x1 invisible cursor for FPS mouse-look locking
        let blank_cursor = unsafe {
            let pm = (x11.XCreatePixmap)(display, window, 1, 1, 1);
            let mut black: XColor = std::mem::zeroed();
            let cur = (x11.XCreatePixmapCursor)(display, pm, pm, &mut black, &mut black, 0, 0);
            (x11.XFreePixmap)(display, pm);
            cur
        };

        Ok(Self {
            backend: Backend::X11 {
                _lib_x11: lib_x11,
                _lib_gl: lib_gl,
                x11,
                glx,
                display,
                window,
                colormap,
                context,
                wm_delete_window,
                last_mouse: None,
                cursor_locked: false,
                blank_cursor,
            },
            gl,
            width,
            height,
            headless: false,
        })
    }

    fn new_headless_egl(width: u32, height: u32) -> Result<Self, String> {
        let lib_egl = DynLib::open(&["libEGL.so.1", "libEGL.so"])?;
        let lib_gl = DynLib::open(&["libGL.so.1", "libGL.so"])?;

        let egl = unsafe {
            EglFns {
                eglGetDisplay: lib_egl.sym("eglGetDisplay")?,
                eglInitialize: lib_egl.sym("eglInitialize")?,
                eglTerminate: lib_egl.sym("eglTerminate")?,
                eglBindAPI: lib_egl.sym("eglBindAPI")?,
                eglChooseConfig: lib_egl.sym("eglChooseConfig")?,
                eglCreatePbufferSurface: lib_egl.sym("eglCreatePbufferSurface")?,
                eglDestroySurface: lib_egl.sym("eglDestroySurface")?,
                eglCreateContext: lib_egl.sym("eglCreateContext")?,
                eglDestroyContext: lib_egl.sym("eglDestroyContext")?,
                eglMakeCurrent: lib_egl.sym("eglMakeCurrent")?,
                eglGetProcAddress: lib_egl.sym("eglGetProcAddress")?,
            }
        };

        // Prefer surfaceless Mesa display so headless works with no X11/Wayland display server.
        let mut display: EGLDisplay = ptr::null_mut();
        let ext_name = CString::new("eglGetPlatformDisplayEXT").unwrap();
        let ext_ptr = unsafe { (egl.eglGetProcAddress)(ext_name.as_ptr()) };
        if !ext_ptr.is_null() {
            type GetPlatDpyFn = unsafe extern "C" fn(EGLenum, *mut c_void, *const EGLint) -> EGLDisplay;
            let get_plat_dpy: GetPlatDpyFn = unsafe { std::mem::transmute(ext_ptr) };
            let cand = unsafe { get_plat_dpy(EGL_PLATFORM_SURFACELESS_MESA, ptr::null_mut(), ptr::null()) };
            if !cand.is_null() {
                let mut maj = 0;
                let mut min = 0;
                if unsafe { (egl.eglInitialize)(cand, &mut maj, &mut min) } != 0 {
                    display = cand;
                }
            }
        }
        if display.is_null() {
            let cand = unsafe { (egl.eglGetDisplay)(ptr::null_mut()) };
            if cand.is_null() {
                return Err("eglGetDisplay returned EGL_NO_DISPLAY".into());
            }
            let mut maj = 0;
            let mut min = 0;
            if unsafe { (egl.eglInitialize)(cand, &mut maj, &mut min) } == 0 {
                return Err("eglInitialize failed".into());
            }
            display = cand;
        }

        if unsafe { (egl.eglBindAPI)(EGL_OPENGL_API) } == 0 {
            return Err("eglBindAPI(EGL_OPENGL_API) failed".into());
        }

        let cfg_attribs: [EGLint; 15] = [
            EGL_SURFACE_TYPE, EGL_PBUFFER_BIT,
            EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT,
            EGL_RED_SIZE, 8,
            EGL_GREEN_SIZE, 8,
            EGL_BLUE_SIZE, 8,
            EGL_ALPHA_SIZE, 8,
            EGL_DEPTH_SIZE, 24,
            EGL_NONE,
        ];
        let mut cfg: EGLConfig = ptr::null_mut();
        let mut num_cfg: EGLint = 0;
        let ok = unsafe { (egl.eglChooseConfig)(display, cfg_attribs.as_ptr(), &mut cfg, 1, &mut num_cfg) };
        if ok == 0 || num_cfg == 0 {
            // Fallback for surfaceless configs without PBUFFER_BIT
            let cfg_attribs2: [EGLint; 13] = [
                EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT,
                EGL_RED_SIZE, 8,
                EGL_GREEN_SIZE, 8,
                EGL_BLUE_SIZE, 8,
                EGL_ALPHA_SIZE, 8,
                EGL_DEPTH_SIZE, 24,
                EGL_NONE,
            ];
            let ok2 = unsafe { (egl.eglChooseConfig)(display, cfg_attribs2.as_ptr(), &mut cfg, 1, &mut num_cfg) };
            if ok2 == 0 || num_cfg == 0 {
                return Err("eglChooseConfig found no OpenGL config".into());
            }
        }

        let ctx_attribs: [EGLint; 7] = [
            EGL_CONTEXT_MAJOR_VERSION, 3,
            EGL_CONTEXT_MINOR_VERSION, 3,
            EGL_CONTEXT_OPENGL_PROFILE_MASK, EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT,
            EGL_NONE,
        ];
        let context = unsafe { (egl.eglCreateContext)(display, cfg, ptr::null_mut(), ctx_attribs.as_ptr()) };
        if context.is_null() {
            return Err("eglCreateContext failed for OpenGL 3.3 Core".into());
        }

        // Try surfaceless makeCurrent first; fall back to a tiny pbuffer surface if needed.
        let mut surface: EGLSurface = ptr::null_mut();
        if unsafe { (egl.eglMakeCurrent)(display, ptr::null_mut(), ptr::null_mut(), context) } == 0 {
            let pb_attribs: [EGLint; 5] = [EGL_WIDTH, 16, EGL_HEIGHT, 16, EGL_NONE];
            surface = unsafe { (egl.eglCreatePbufferSurface)(display, cfg, pb_attribs.as_ptr()) };
            if surface.is_null() {
                return Err("eglCreatePbufferSurface failed".into());
            }
            if unsafe { (egl.eglMakeCurrent)(display, surface, surface, context) } == 0 {
                return Err("eglMakeCurrent failed".into());
            }
        }

        let gl = GlFns::load(|name| {
            let c = CString::new(name).unwrap();
            let p = unsafe { (egl.eglGetProcAddress)(c.as_ptr()) };
            if !p.is_null() { p } else { lib_gl.sym_raw(name) }
        })?;

        // Create offscreen RGBA8 + Depth24 FBO for deterministic headless rendering.
        let mut fbo: GLuint = 0;
        let mut color_tex: GLuint = 0;
        let mut depth_rb: GLuint = 0;
        unsafe {
            (gl.glGenFramebuffers)(1, &mut fbo);
            (gl.glBindFramebuffer)(GL_FRAMEBUFFER, fbo);

            (gl.glGenTextures)(1, &mut color_tex);
            (gl.glBindTexture)(GL_TEXTURE_2D, color_tex);
            (gl.glTexImage2D)(
                GL_TEXTURE_2D,
                0,
                GL_RGBA8 as c_int,
                width as c_int,
                height as c_int,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                ptr::null(),
            );
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            (gl.glFramebufferTexture2D)(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, color_tex, 0);

            (gl.glGenRenderbuffers)(1, &mut depth_rb);
            (gl.glBindRenderbuffer)(GL_RENDERBUFFER, depth_rb);
            (gl.glRenderbufferStorage)(GL_RENDERBUFFER, GL_DEPTH_COMPONENT24, width as c_int, height as c_int);
            (gl.glFramebufferRenderbuffer)(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, depth_rb);

            let status = (gl.glCheckFramebufferStatus)(GL_FRAMEBUFFER);
            if status != GL_FRAMEBUFFER_COMPLETE {
                return Err(format!("headless FBO incomplete: 0x{status:x}"));
            }
        }

        Ok(Self {
            backend: Backend::HeadlessEgl {
                _lib_egl: lib_egl,
                _lib_gl: lib_gl,
                egl,
                display,
                surface,
                context,
                fbo,
                color_tex,
                depth_rb,
            },
            gl,
            width,
            height,
            headless: true,
        })
    }

    /// Returns the default framebuffer handle (`0` for X11 window, offscreen FBO id for headless).
    pub fn default_framebuffer(&self) -> GLuint {
        match &self.backend {
            Backend::X11 { .. } => 0,
            Backend::HeadlessEgl { fbo, .. } => *fbo,
        }
    }

    /// Locks and hides the cursor at window center for smooth 360-degree FPS mouse look.
    pub fn set_cursor_locked(&mut self, locked: bool) {
        let Backend::X11 {
            x11,
            display,
            window,
            last_mouse,
            cursor_locked,
            blank_cursor,
            ..
        } = &mut self.backend
        else {
            return;
        };
        if *cursor_locked == locked {
            return;
        }
        *cursor_locked = locked;
        unsafe {
            if locked {
                (x11.XDefineCursor)(*display, *window, *blank_cursor);
                let cx = (self.width / 2) as c_int;
                let cy = (self.height / 2) as c_int;
                (x11.XWarpPointer)(*display, 0, *window, 0, 0, 0, 0, cx, cy);
                (x11.XFlush)(*display);
                *last_mouse = Some(Vec2::new(cx as f32, cy as f32));
            } else {
                (x11.XUndefineCursor)(*display, *window);
                (x11.XFlush)(*display);
                *last_mouse = None;
            }
        }
    }

    pub fn is_cursor_locked(&self) -> bool {
        match &self.backend {
            Backend::X11 { cursor_locked, .. } => *cursor_locked,
            Backend::HeadlessEgl { .. } => false,
        }
    }

    pub fn poll_events(&mut self, input: &mut InputState) {
        let Backend::X11 {
            x11,
            display,
            window,
            wm_delete_window,
            last_mouse,
            cursor_locked,
            ..
        } = &mut self.backend
        else {
            return;
        };

        let cx = (self.width / 2) as c_int;
        let cy = (self.height / 2) as c_int;
        let mut need_recenter = false;

        unsafe {
            while (x11.XPending)(*display) > 0 {
                let mut ev: XEvent = std::mem::zeroed();
                (x11.XNextEvent)(*display, &mut ev);
                match ev.type_ {
                    KEY_PRESS => {
                        let mut ke = ev.xkey;
                        let sym = (x11.XLookupKeysym)(&mut ke, 0);
                        if let Some(k) = keysym_to_key(sym) {
                            input.set_key(k, true);
                        }
                    }
                    KEY_RELEASE => {
                        let mut ke = ev.xkey;
                        let sym = (x11.XLookupKeysym)(&mut ke, 0);
                        if let Some(k) = keysym_to_key(sym) {
                            input.set_key(k, false);
                        }
                    }
                    BUTTON_PRESS => {
                        let b = ev.xbutton.button;
                        match b {
                            1 => input.set_mouse_button(MouseButton::Left, true),
                            2 => input.set_mouse_button(MouseButton::Middle, true),
                            3 => input.set_mouse_button(MouseButton::Right, true),
                            4 => input.scroll_delta += 1.0,
                            5 => input.scroll_delta -= 1.0,
                            _ => {}
                        }
                    }
                    BUTTON_RELEASE => {
                        let b = ev.xbutton.button;
                        match b {
                            1 => input.set_mouse_button(MouseButton::Left, false),
                            2 => input.set_mouse_button(MouseButton::Middle, false),
                            3 => input.set_mouse_button(MouseButton::Right, false),
                            _ => {}
                        }
                    }
                    MOTION_NOTIFY => {
                        let pos = Vec2::new(ev.xmotion.x as f32, ev.xmotion.y as f32);
                        if *cursor_locked {
                            let dx = ev.xmotion.x - cx;
                            let dy = ev.xmotion.y - cy;
                            if dx != 0 || dy != 0 {
                                input.mouse_delta = input.mouse_delta + Vec2::new(dx as f32, dy as f32);
                                need_recenter = true;
                            }
                            input.mouse_pos = Vec2::new(cx as f32, cy as f32);
                        } else {
                            if let Some(prev) = *last_mouse {
                                input.mouse_delta = input.mouse_delta + (pos - prev);
                            }
                            *last_mouse = Some(pos);
                            input.mouse_pos = pos;
                        }
                    }
                    CONFIGURE_NOTIFY => {
                        let w = ev.xconfigure.width.max(1) as u32;
                        let h = ev.xconfigure.height.max(1) as u32;
                        self.width = w;
                        self.height = h;
                    }
                    FOCUS_IN => input.focused = true,
                    FOCUS_OUT => {
                        input.focused = false;
                        input.clear_all();
                    }
                    CLIENT_MESSAGE => {
                        if ev.xclient.data[0] as XAtom == *wm_delete_window {
                            input.quit_requested = true;
                        }
                    }
                    _ => {}
                }
            }
            if need_recenter && *cursor_locked && input.focused {
                (x11.XWarpPointer)(*display, 0, *window, 0, 0, 0, 0, cx, cy);
                (x11.XFlush)(*display);
            }
        }
    }

    pub fn swap_buffers(&self) {
        match &self.backend {
            Backend::X11 { glx, display, window, .. } => unsafe {
                (glx.glXSwapBuffers)(*display, *window);
            },
            Backend::HeadlessEgl { .. } => unsafe {
                (self.gl.glFinish)();
            },
        }
    }
}

impl Drop for Platform {
    fn drop(&mut self) {
        match &self.backend {
            Backend::X11 {
                x11,
                glx,
                display,
                window,
                colormap,
                context,
                blank_cursor,
                ..
            } => unsafe {
                (glx.glXMakeCurrent)(*display, 0, ptr::null_mut());
                (glx.glXDestroyContext)(*display, *context);
                (x11.XFreeCursor)(*display, *blank_cursor);
                (x11.XDestroyWindow)(*display, *window);
                (x11.XFreeColormap)(*display, *colormap);
                (x11.XCloseDisplay)(*display);
            },
            Backend::HeadlessEgl {
                egl,
                display,
                surface,
                context,
                fbo,
                color_tex,
                depth_rb,
                ..
            } => unsafe {
                (self.gl.glDeleteFramebuffers)(1, fbo);
                (self.gl.glDeleteTextures)(1, color_tex);
                (self.gl.glDeleteRenderbuffers)(1, depth_rb);
                (egl.eglMakeCurrent)(*display, ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
                (egl.eglDestroyContext)(*display, *context);
                if !surface.is_null() {
                    (egl.eglDestroySurface)(*display, *surface);
                }
                (egl.eglTerminate)(*display);
            },
        }
    }
}

fn keysym_to_key(sym: c_ulong) -> Option<Key> {
    match sym {
        0x0061 | 0x0041 => Some(Key::A),
        0x0062 | 0x0042 => Some(Key::B),
        0x0063 | 0x0043 => Some(Key::C),
        0x0064 | 0x0044 => Some(Key::D),
        0x0065 | 0x0045 => Some(Key::E),
        0x0066 | 0x0046 => Some(Key::F),
        0x0067 | 0x0047 => Some(Key::G),
        0x0068 | 0x0048 => Some(Key::H),
        0x0069 | 0x0049 => Some(Key::I),
        0x006A | 0x004A => Some(Key::J),
        0x006B | 0x004B => Some(Key::K),
        0x006C | 0x004C => Some(Key::L),
        0x006D | 0x004D => Some(Key::M),
        0x006E | 0x004E => Some(Key::N),
        0x006F | 0x004F => Some(Key::O),
        0x0070 | 0x0050 => Some(Key::P),
        0x0071 | 0x0051 => Some(Key::Q),
        0x0072 | 0x0052 => Some(Key::R),
        0x0073 | 0x0053 => Some(Key::S),
        0x0074 | 0x0054 => Some(Key::T),
        0x0075 | 0x0055 => Some(Key::U),
        0x0076 | 0x0056 => Some(Key::V),
        0x0077 | 0x0057 => Some(Key::W),
        0x0078 | 0x0058 => Some(Key::X),
        0x0079 | 0x0059 => Some(Key::Y),
        0x007A | 0x005A => Some(Key::Z),
        0x0030 => Some(Key::Num0),
        0x0031 => Some(Key::Num1),
        0x0032 => Some(Key::Num2),
        0x0033 => Some(Key::Num3),
        0x0034 => Some(Key::Num4),
        0x0035 => Some(Key::Num5),
        0x0036 => Some(Key::Num6),
        0x0037 => Some(Key::Num7),
        0x0038 => Some(Key::Num8),
        0x0039 => Some(Key::Num9),
        0x0020 => Some(Key::Space),
        0xFF0D => Some(Key::Enter),
        0xFF1B => Some(Key::Escape),
        0xFF09 => Some(Key::Tab),
        0xFF08 => Some(Key::Backspace),
        0xFFE1 => Some(Key::LShift),
        0xFFE2 => Some(Key::RShift),
        0xFFE3 => Some(Key::LCtrl),
        0xFFE4 => Some(Key::RCtrl),
        0xFFE9 => Some(Key::LAlt),
        0xFF51 => Some(Key::Left),
        0xFF52 => Some(Key::Up),
        0xFF53 => Some(Key::Right),
        0xFF54 => Some(Key::Down),
        0xFFBE => Some(Key::F1),
        0xFFBF => Some(Key::F2),
        0xFFC0 => Some(Key::F3),
        0xFFC1 => Some(Key::F4),
        0xFFC2 => Some(Key::F5),
        0xFFC3 => Some(Key::F6),
        0xFFC4 => Some(Key::F7),
        0xFFC5 => Some(Key::F8),
        0xFFC6 => Some(Key::F9),
        0xFFC7 => Some(Key::F10),
        0xFFC8 => Some(Key::F11),
        0xFFC9 => Some(Key::F12),
        _ => None,
    }
}
