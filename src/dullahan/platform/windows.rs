//! Zero-dependency Windows backend (Win32 + WGL OpenGL 3.3 Core).
//!
//! - Windowed mode: `user32.dll` + `gdi32.dll` + `opengl32.dll` (WGL -> OpenGL 3.3 Core)
//! - Headless mode: hidden Win32 window + OpenGL 3.3 Core offscreen FBO (works cleanly with `--headless`)
//! - Native FPS cursor lock via `ShowCursor` + `ClipCursor` + center recenter with incremental deltas.

#![allow(non_snake_case, non_camel_case_types, dead_code)]

use crate::dullahan::math::Vec2;
use crate::dullahan::platform::{
    GL_COLOR_ATTACHMENT0, GL_DEPTH_ATTACHMENT, GL_DEPTH_COMPONENT24, GL_FRAMEBUFFER,
    GL_FRAMEBUFFER_COMPLETE, GL_LINEAR, GL_RENDERBUFFER, GL_RGBA, GL_RGBA8, GL_TEXTURE_2D,
    GL_TEXTURE_MAG_FILTER, GL_TEXTURE_MIN_FILTER, GL_UNSIGNED_BYTE, GLint, GLsizei, GLuint,
    GlFns, InputState, Key, MouseButton,
};
use std::ffi::{CString, c_char, c_int, c_long, c_short, c_uchar, c_uint, c_ulong, c_ushort, c_void};
use std::ptr;

type HWND = *mut c_void;
type HDC = *mut c_void;
type HGLRC = *mut c_void;
type HINSTANCE = *mut c_void;
type HMODULE = *mut c_void;
type HICON = *mut c_void;
type HCURSOR = *mut c_void;
type HBRUSH = *mut c_void;
type HMENU = *mut c_void;
type BOOL = c_int;
type UINT = c_uint;
type DWORD = c_ulong;
type WORD = c_ushort;
type BYTE = c_uchar;
type LONG = c_long;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type ATOM = WORD;

#[repr(C)]
struct WNDCLASSA {
    style: UINT,
    lpfnWndProc: unsafe extern "system" fn(HWND, UINT, WPARAM, LPARAM) -> LRESULT,
    cbClsExtra: c_int,
    cbWndExtra: c_int,
    hInstance: HINSTANCE,
    hIcon: HICON,
    hCursor: HCURSOR,
    hbrBackground: HBRUSH,
    lpszMenuName: *const c_char,
    lpszClassName: *const c_char,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct POINT {
    x: LONG,
    y: LONG,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct RECT {
    left: LONG,
    top: LONG,
    right: LONG,
    bottom: LONG,
}

#[repr(C)]
struct MSG {
    hwnd: HWND,
    message: UINT,
    wParam: WPARAM,
    lParam: LPARAM,
    time: DWORD,
    pt: POINT,
}

#[repr(C)]
struct PIXELFORMATDESCRIPTOR {
    nSize: WORD,
    nVersion: WORD,
    dwFlags: DWORD,
    iPixelType: BYTE,
    cColorBits: BYTE,
    cRedBits: BYTE,
    cRedShift: BYTE,
    cGreenBits: BYTE,
    cGreenShift: BYTE,
    cBlueBits: BYTE,
    cBlueShift: BYTE,
    cAlphaBits: BYTE,
    cAlphaShift: BYTE,
    cAccumBits: BYTE,
    cAccumRedBits: BYTE,
    cAccumGreenBits: BYTE,
    cAccumBlueBits: BYTE,
    cAccumAlphaBits: BYTE,
    cDepthBits: BYTE,
    cStencilBits: BYTE,
    cAuxBuffers: BYTE,
    iLayerType: BYTE,
    bReserved: BYTE,
    dwLayerMask: DWORD,
    dwVisibleMask: DWORD,
    dwDamageMask: DWORD,
}

const CS_OWNDC: UINT = 0x0020;
const WS_OVERLAPPEDWINDOW: DWORD = 0x00CF_0000;
const WS_CLIPSIBLINGS: DWORD = 0x0400_0000;
const WS_CLIPCHILDREN: DWORD = 0x0200_0000;
const CW_USEDEFAULT: c_int = -0x8000_0000;
const SW_SHOW: c_int = 5;
const PM_REMOVE: UINT = 0x0001;
const IDC_ARROW: *const c_char = 32512 as *const c_char;

const PFD_DOUBLEBUFFER: DWORD = 0x0000_0001;
const PFD_DRAW_TO_WINDOW: DWORD = 0x0000_0004;
const PFD_SUPPORT_OPENGL: DWORD = 0x0000_0020;
const PFD_TYPE_RGBA: BYTE = 0;
const PFD_MAIN_PLANE: BYTE = 0;

const WGL_CONTEXT_MAJOR_VERSION_ARB: c_int = 0x2091;
const WGL_CONTEXT_MINOR_VERSION_ARB: c_int = 0x2092;
const WGL_CONTEXT_PROFILE_MASK_ARB: c_int = 0x9126;
const WGL_CONTEXT_CORE_PROFILE_BIT_ARB: c_int = 0x0001;

const WM_SIZE: UINT = 0x0005;
const WM_SETFOCUS: UINT = 0x0007;
const WM_KILLFOCUS: UINT = 0x0008;
const WM_CLOSE: UINT = 0x0010;
const WM_KEYDOWN: UINT = 0x0100;
const WM_KEYUP: UINT = 0x0101;
const WM_SYSKEYDOWN: UINT = 0x0104;
const WM_SYSKEYUP: UINT = 0x0105;
const WM_MOUSEMOVE: UINT = 0x0200;
const WM_LBUTTONDOWN: UINT = 0x0201;
const WM_LBUTTONUP: UINT = 0x0202;
const WM_RBUTTONDOWN: UINT = 0x0204;
const WM_RBUTTONUP: UINT = 0x0205;
const WM_MBUTTONDOWN: UINT = 0x0207;
const WM_MBUTTONUP: UINT = 0x0208;
const WM_MOUSEWHEEL: UINT = 0x020A;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleA(lpModuleName: *const c_char) -> HMODULE;
    fn LoadLibraryA(lpLibFileName: *const c_char) -> HMODULE;
    fn FreeLibrary(hLibModule: HMODULE) -> BOOL;
    fn GetProcAddress(hModule: HMODULE, lpProcName: *const c_char) -> *const c_void;
}

#[link(name = "user32")]
unsafe extern "system" {
    fn SetProcessDPIAware() -> BOOL;
    fn RegisterClassA(lpWndClass: *const WNDCLASSA) -> ATOM;
    fn CreateWindowExA(
        dwExStyle: DWORD,
        lpClassName: *const c_char,
        lpWindowName: *const c_char,
        dwStyle: DWORD,
        X: c_int,
        Y: c_int,
        nWidth: c_int,
        nHeight: c_int,
        hWndParent: HWND,
        hMenu: HMENU,
        hInstance: HINSTANCE,
        lpParam: *mut c_void,
    ) -> HWND;
    fn DestroyWindow(hWnd: HWND) -> BOOL;
    fn ShowWindow(hWnd: HWND, nCmdShow: c_int) -> BOOL;
    fn GetDC(hWnd: HWND) -> HDC;
    fn ReleaseDC(hWnd: HWND, hDC: HDC) -> c_int;
    fn DefWindowProcA(hWnd: HWND, Msg: UINT, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
    fn PeekMessageA(lpMsg: *mut MSG, hWnd: HWND, wMsgFilterMin: UINT, wMsgFilterMax: UINT, wRemoveMsg: UINT) -> BOOL;
    fn TranslateMessage(lpMsg: *const MSG) -> BOOL;
    fn DispatchMessageA(lpMsg: *const MSG) -> LRESULT;
    fn AdjustWindowRectEx(lpRect: *mut RECT, dwStyle: DWORD, bMenu: BOOL, dwExStyle: DWORD) -> BOOL;
    fn GetClientRect(hWnd: HWND, lpRect: *mut RECT) -> BOOL;
    fn ClientToScreen(hWnd: HWND, lpPoint: *mut POINT) -> BOOL;
    fn LoadCursorA(hInstance: HINSTANCE, lpCursorName: *const c_char) -> HCURSOR;
    fn ShowCursor(bShow: BOOL) -> c_int;
    fn ClipCursor(lpRect: *const RECT) -> BOOL;
    fn SetCursorPos(X: c_int, Y: c_int) -> BOOL;
    fn SetCapture(hWnd: HWND) -> HWND;
    fn ReleaseCapture() -> BOOL;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn ChoosePixelFormat(hdc: HDC, ppfd: *const PIXELFORMATDESCRIPTOR) -> c_int;
    fn SetPixelFormat(hdc: HDC, format: c_int, ppfd: *const PIXELFORMATDESCRIPTOR) -> BOOL;
    fn SwapBuffers(hdc: HDC) -> BOOL;
}

#[link(name = "opengl32")]
unsafe extern "system" {
    fn wglCreateContext(hdc: HDC) -> HGLRC;
    fn wglDeleteContext(hglrc: HGLRC) -> BOOL;
    fn wglMakeCurrent(hdc: HDC, hglrc: HGLRC) -> BOOL;
    fn wglGetProcAddress(lpszProc: *const c_char) -> *const c_void;
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: UINT, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_CLOSE {
        // Intercept WM_CLOSE so our message loop receives it cleanly as quit_requested
        return 0;
    }
    unsafe { DefWindowProcA(hwnd, msg, wparam, lparam) }
}

pub struct Platform {
    hwnd: HWND,
    hdc: HDC,
    hglrc: HGLRC,
    opengl32: HMODULE,
    fbo: GLuint,
    color_tex: GLuint,
    depth_rb: GLuint,
    last_mouse: Option<Vec2>,
    cursor_locked: bool,
    ignore_warp: bool,
    pub gl: GlFns,
    pub width: u32,
    pub height: u32,
    pub headless: bool,
}

impl Platform {
    pub fn new(title: &str, width: u32, height: u32, headless: bool) -> Result<Self, String> {
        unsafe {
            SetProcessDPIAware();
            let hinst = GetModuleHandleA(ptr::null());
            let class_name = CString::new("DullahanWin32Class").unwrap();
            let wc = WNDCLASSA {
                style: CS_OWNDC,
                lpfnWndProc: wnd_proc,
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinst,
                hIcon: ptr::null_mut(),
                hCursor: LoadCursorA(ptr::null_mut(), IDC_ARROW),
                hbrBackground: ptr::null_mut(),
                lpszMenuName: ptr::null(),
                lpszClassName: class_name.as_ptr(),
            };
            RegisterClassA(&wc);

            let style = WS_OVERLAPPEDWINDOW | WS_CLIPSIBLINGS | WS_CLIPCHILDREN;
            let mut rect = RECT { left: 0, top: 0, right: width as LONG, bottom: height as LONG };
            AdjustWindowRectEx(&mut rect, style, 0, 0);
            let win_w = (rect.right - rect.left) as c_int;
            let win_h = (rect.bottom - rect.top) as c_int;

            let title_c = CString::new(title).unwrap_or_else(|_| CString::new("Dullahan").unwrap());
            let hwnd = CreateWindowExA(
                0,
                class_name.as_ptr(),
                title_c.as_ptr(),
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                win_w,
                win_h,
                ptr::null_mut(),
                ptr::null_mut(),
                hinst,
                ptr::null_mut(),
            );
            if hwnd.is_null() {
                return Err("Win32 CreateWindowExA failed".into());
            }

            let hdc = GetDC(hwnd);
            if hdc.is_null() {
                DestroyWindow(hwnd);
                return Err("Win32 GetDC failed".into());
            }

            let mut pfd: PIXELFORMATDESCRIPTOR = std::mem::zeroed();
            pfd.nSize = std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as WORD;
            pfd.nVersion = 1;
            pfd.dwFlags = PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER;
            pfd.iPixelType = PFD_TYPE_RGBA;
            pfd.cColorBits = 32;
            pfd.cDepthBits = 24;
            pfd.cStencilBits = 8;
            pfd.iLayerType = PFD_MAIN_PLANE;

            let pf = ChoosePixelFormat(hdc, &pfd);
            if pf == 0 || SetPixelFormat(hdc, pf, &pfd) == 0 {
                ReleaseDC(hwnd, hdc);
                DestroyWindow(hwnd);
                return Err("Win32 SetPixelFormat failed".into());
            }

            let dummy_rc = wglCreateContext(hdc);
            if dummy_rc.is_null() || wglMakeCurrent(hdc, dummy_rc) == 0 {
                if !dummy_rc.is_null() {
                    wglDeleteContext(dummy_rc);
                }
                ReleaseDC(hwnd, hdc);
                DestroyWindow(hwnd);
                return Err("WGL bootstrap context creation failed".into());
            }

            // Upgrade to OpenGL 3.3 Core Profile via wglCreateContextAttribsARB
            let mut hglrc = dummy_rc;
            let create_arb_name = CString::new("wglCreateContextAttribsARB").unwrap();
            let create_arb_ptr = wglGetProcAddress(create_arb_name.as_ptr());
            if is_valid_wgl_ptr(create_arb_ptr) {
                type WglCreateCtxArb = unsafe extern "system" fn(HDC, HGLRC, *const c_int) -> HGLRC;
                let create_arb: WglCreateCtxArb = std::mem::transmute(create_arb_ptr);
                let attribs = [
                    WGL_CONTEXT_MAJOR_VERSION_ARB, 3,
                    WGL_CONTEXT_MINOR_VERSION_ARB, 3,
                    WGL_CONTEXT_PROFILE_MASK_ARB, WGL_CONTEXT_CORE_PROFILE_BIT_ARB,
                    0,
                ];
                let core_rc = create_arb(hdc, ptr::null_mut(), attribs.as_ptr());
                if !core_rc.is_null() && wglMakeCurrent(hdc, core_rc) != 0 {
                    wglDeleteContext(dummy_rc);
                    hglrc = core_rc;
                }
            }

            // Enable VSync in windowed mode so laptops stay cool and smooth
            if !headless {
                let swap_name = CString::new("wglSwapIntervalEXT").unwrap();
                let swap_ptr = wglGetProcAddress(swap_name.as_ptr());
                if is_valid_wgl_ptr(swap_ptr) {
                    type WglSwapInterval = unsafe extern "system" fn(c_int) -> BOOL;
                    let swap_ext: WglSwapInterval = std::mem::transmute(swap_ptr);
                    swap_ext(1);
                }
            }

            let gl32_name = CString::new("opengl32.dll").unwrap();
            let opengl32 = LoadLibraryA(gl32_name.as_ptr());
            let gl = GlFns::load(|name| {
                let c = CString::new(name).unwrap();
                let p = wglGetProcAddress(c.as_ptr());
                if is_valid_wgl_ptr(p) {
                    p
                } else if !opengl32.is_null() {
                    GetProcAddress(opengl32, c.as_ptr())
                } else {
                    ptr::null()
                }
            })?;

            let mut fbo = 0;
            let mut color_tex = 0;
            let mut depth_rb = 0;
            if headless {
                (gl.glGenFramebuffers)(1, &mut fbo);
                (gl.glBindFramebuffer)(GL_FRAMEBUFFER, fbo);

                (gl.glGenTextures)(1, &mut color_tex);
                (gl.glBindTexture)(GL_TEXTURE_2D, color_tex);
                (gl.glTexImage2D)(
                    GL_TEXTURE_2D,
                    0,
                    GL_RGBA8 as GLint,
                    width as GLsizei,
                    height as GLsizei,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    ptr::null(),
                );
                (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
                (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
                (gl.glFramebufferTexture2D)(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, color_tex, 0);

                (gl.glGenRenderbuffers)(1, &mut depth_rb);
                (gl.glBindRenderbuffer)(GL_RENDERBUFFER, depth_rb);
                (gl.glRenderbufferStorage)(GL_RENDERBUFFER, GL_DEPTH_COMPONENT24, width as GLsizei, height as GLsizei);
                (gl.glFramebufferRenderbuffer)(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, depth_rb);

                let status = (gl.glCheckFramebufferStatus)(GL_FRAMEBUFFER);
                if status != GL_FRAMEBUFFER_COMPLETE {
                    return Err(format!("Win32 headless FBO incomplete: 0x{status:x}"));
                }
            } else {
                ShowWindow(hwnd, SW_SHOW);
            }

            Ok(Self {
                hwnd,
                hdc,
                hglrc,
                opengl32,
                fbo,
                color_tex,
                depth_rb,
                last_mouse: None,
                cursor_locked: false,
                ignore_warp: false,
                gl,
                width,
                height,
                headless,
            })
        }
    }

    pub fn default_framebuffer(&self) -> GLuint {
        if self.headless { self.fbo } else { 0 }
    }

    fn recenter_cursor(&mut self) {
        if self.headless {
            return;
        }
        let cx = (self.width / 2) as LONG;
        let cy = (self.height / 2) as LONG;
        let mut pt = POINT { x: cx, y: cy };
        unsafe {
            ClientToScreen(self.hwnd, &mut pt);
            SetCursorPos(pt.x as c_int, pt.y as c_int);
        }
        self.last_mouse = Some(Vec2::new(cx as f32, cy as f32));
    }

    pub fn set_cursor_locked(&mut self, locked: bool) {
        if self.headless || self.cursor_locked == locked {
            return;
        }
        self.cursor_locked = locked;
        unsafe {
            if locked {
                while ShowCursor(0) >= 0 {}
                let mut r = RECT::default();
                GetClientRect(self.hwnd, &mut r);
                let mut tl = POINT { x: r.left, y: r.top };
                let mut br = POINT { x: r.right, y: r.bottom };
                ClientToScreen(self.hwnd, &mut tl);
                ClientToScreen(self.hwnd, &mut br);
                let clip = RECT { left: tl.x, top: tl.y, right: br.x, bottom: br.y };
                ClipCursor(&clip);
                self.recenter_cursor();
                self.ignore_warp = true;
            } else {
                ClipCursor(ptr::null());
                while ShowCursor(1) < 0 {}
                self.last_mouse = None;
                self.ignore_warp = false;
            }
        }
    }

    pub fn is_cursor_locked(&self) -> bool {
        self.cursor_locked
    }

    pub fn poll_events(&mut self, input: &mut InputState) {
        if self.headless {
            return;
        }
        let cx = (self.width / 2) as i32;
        let cy = (self.height / 2) as i32;
        let center = Vec2::new(cx as f32, cy as f32);
        let mut need_recenter = false;

        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageA(&mut msg, self.hwnd, 0, 0, PM_REMOVE) != 0 {
                match msg.message {
                    WM_CLOSE => input.quit_requested = true,
                    WM_SIZE => {
                        let w = (msg.lParam & 0xFFFF) as u32;
                        let h = ((msg.lParam >> 16) & 0xFFFF) as u32;
                        if w > 0 && h > 0 {
                            self.width = w;
                            self.height = h;
                        }
                    }
                    WM_SETFOCUS => input.focused = true,
                    WM_KILLFOCUS => {
                        input.focused = false;
                        input.clear_all();
                        if self.cursor_locked {
                            self.set_cursor_locked(false);
                        }
                    }
                    WM_KEYDOWN | WM_SYSKEYDOWN => {
                        if let Some(k) = vk_to_key(msg.wParam, msg.lParam) {
                            input.set_key(k, true);
                        }
                    }
                    WM_KEYUP | WM_SYSKEYUP => {
                        if let Some(k) = vk_to_key(msg.wParam, msg.lParam) {
                            input.set_key(k, false);
                        }
                    }
                    WM_LBUTTONDOWN => {
                        SetCapture(self.hwnd);
                        input.set_mouse_button(MouseButton::Left, true);
                    }
                    WM_LBUTTONUP => {
                        ReleaseCapture();
                        input.set_mouse_button(MouseButton::Left, false);
                    }
                    WM_RBUTTONDOWN => input.set_mouse_button(MouseButton::Right, true),
                    WM_RBUTTONUP => input.set_mouse_button(MouseButton::Right, false),
                    WM_MBUTTONDOWN => input.set_mouse_button(MouseButton::Middle, true),
                    WM_MBUTTONUP => input.set_mouse_button(MouseButton::Middle, false),
                    WM_MOUSEWHEEL => {
                        let delta = ((msg.wParam >> 16) as c_short) as f32 / 120.0;
                        input.scroll_delta += delta;
                    }
                    WM_MOUSEMOVE => {
                        let x = (msg.lParam as i16) as i32;
                        let y = ((msg.lParam >> 16) as i16) as i32;
                        let pos = Vec2::new(x as f32, y as f32);
                        if self.cursor_locked {
                            if x == cx && y == cy {
                                self.last_mouse = Some(center);
                                self.ignore_warp = false;
                            } else if self.ignore_warp {
                                self.last_mouse = Some(pos);
                            } else {
                                let prev = self.last_mouse.unwrap_or(center);
                                input.mouse_delta += pos - prev;
                                self.last_mouse = Some(pos);
                                need_recenter = true;
                            }
                            input.mouse_pos = center;
                        } else {
                            if let Some(prev) = self.last_mouse {
                                input.mouse_delta += pos - prev;
                            }
                            self.last_mouse = Some(pos);
                            input.mouse_pos = pos;
                        }
                    }
                    _ => {}
                }
                TranslateMessage(&msg);
                DispatchMessageA(&msg);
            }
        }
        if need_recenter && self.cursor_locked && input.focused {
            self.recenter_cursor();
        }
    }

    pub fn swap_buffers(&self) {
        unsafe {
            if self.headless {
                (self.gl.glFinish)();
            } else {
                SwapBuffers(self.hdc);
            }
        }
    }
}

impl Drop for Platform {
    fn drop(&mut self) {
        if self.cursor_locked {
            self.set_cursor_locked(false);
        }
        unsafe {
            if self.headless {
                if self.fbo != 0 {
                    (self.gl.glDeleteFramebuffers)(1, &self.fbo);
                }
                if self.color_tex != 0 {
                    (self.gl.glDeleteTextures)(1, &self.color_tex);
                }
                if self.depth_rb != 0 {
                    (self.gl.glDeleteRenderbuffers)(1, &self.depth_rb);
                }
            }
            wglMakeCurrent(ptr::null_mut(), ptr::null_mut());
            if !self.hglrc.is_null() {
                wglDeleteContext(self.hglrc);
            }
            if !self.hdc.is_null() && !self.hwnd.is_null() {
                ReleaseDC(self.hwnd, self.hdc);
            }
            if !self.hwnd.is_null() {
                DestroyWindow(self.hwnd);
            }
            if !self.opengl32.is_null() {
                FreeLibrary(self.opengl32);
            }
        }
    }
}

fn is_valid_wgl_ptr(p: *const c_void) -> bool {
    let v = p as isize;
    v != 0 && v != 1 && v != 2 && v != 3 && v != -1
}

fn vk_to_key(vk: WPARAM, lparam: LPARAM) -> Option<Key> {
    let extended = ((lparam >> 24) & 1) != 0;
    let scancode = ((lparam >> 16) & 0xFF) as u32;
    match vk as u32 {
        0x41 => Some(Key::A), 0x42 => Some(Key::B), 0x43 => Some(Key::C), 0x44 => Some(Key::D),
        0x45 => Some(Key::E), 0x46 => Some(Key::F), 0x47 => Some(Key::G), 0x48 => Some(Key::H),
        0x49 => Some(Key::I), 0x4A => Some(Key::J), 0x4B => Some(Key::K), 0x4C => Some(Key::L),
        0x4D => Some(Key::M), 0x4E => Some(Key::N), 0x4F => Some(Key::O), 0x50 => Some(Key::P),
        0x51 => Some(Key::Q), 0x52 => Some(Key::R), 0x53 => Some(Key::S), 0x54 => Some(Key::T),
        0x55 => Some(Key::U), 0x56 => Some(Key::V), 0x57 => Some(Key::W), 0x58 => Some(Key::X),
        0x59 => Some(Key::Y), 0x5A => Some(Key::Z),
        0x30 => Some(Key::Num0), 0x31 => Some(Key::Num1), 0x32 => Some(Key::Num2),
        0x33 => Some(Key::Num3), 0x34 => Some(Key::Num4), 0x35 => Some(Key::Num5),
        0x36 => Some(Key::Num6), 0x37 => Some(Key::Num7), 0x38 => Some(Key::Num8),
        0x39 => Some(Key::Num9),
        0x20 => Some(Key::Space),
        0x0D => Some(Key::Enter),
        0x1B => Some(Key::Escape),
        0x09 => Some(Key::Tab),
        0x08 => Some(Key::Backspace),
        0x10 => Some(if scancode == 0x36 { Key::RShift } else { Key::LShift }),
        0x11 => Some(if extended { Key::RCtrl } else { Key::LCtrl }),
        0x12 => Some(Key::LAlt),
        0x26 => Some(Key::Up),
        0x28 => Some(Key::Down),
        0x25 => Some(Key::Left),
        0x27 => Some(Key::Right),
        0x70 => Some(Key::F1), 0x71 => Some(Key::F2), 0x72 => Some(Key::F3), 0x73 => Some(Key::F4),
        0x74 => Some(Key::F5), 0x75 => Some(Key::F6), 0x76 => Some(Key::F7), 0x77 => Some(Key::F8),
        0x78 => Some(Key::F9), 0x79 => Some(Key::F10), 0x7A => Some(Key::F11), 0x7B => Some(Key::F12),
        _ => None,
    }
}
