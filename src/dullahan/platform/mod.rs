//! OS-neutral platform types, input state, OpenGL 3.3 Core function table, and backend selection.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub use linux::Platform;

#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "windows")]
pub use windows::Platform;

use crate::dullahan::math::Vec2;
use std::ffi::{c_char, c_float, c_int, c_uchar, c_uint, c_void};

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Key {
    A = 0, B, C, D, E, F, G, H, I, J, K, L, M,
    N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    Num0, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9,
    Space, Enter, Escape, Tab, Backspace,
    LShift, RShift, LCtrl, RCtrl, LAlt,
    Up, Down, Left, Right,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    Count,
}

impl Key {
    pub const COUNT: usize = Key::Count as usize;

    pub fn from_name(name: &str) -> Option<Key> {
        match name.to_ascii_uppercase().as_str() {
            "A" => Some(Key::A), "B" => Some(Key::B), "C" => Some(Key::C), "D" => Some(Key::D),
            "E" => Some(Key::E), "F" => Some(Key::F), "G" => Some(Key::G), "H" => Some(Key::H),
            "I" => Some(Key::I), "J" => Some(Key::J), "K" => Some(Key::K), "L" => Some(Key::L),
            "M" => Some(Key::M), "N" => Some(Key::N), "O" => Some(Key::O), "P" => Some(Key::P),
            "Q" => Some(Key::Q), "R" => Some(Key::R), "S" => Some(Key::S), "T" => Some(Key::T),
            "U" => Some(Key::U), "V" => Some(Key::V), "W" => Some(Key::W), "X" => Some(Key::X),
            "Y" => Some(Key::Y), "Z" => Some(Key::Z),
            "0" | "NUM0" => Some(Key::Num0), "1" | "NUM1" => Some(Key::Num1),
            "2" | "NUM2" => Some(Key::Num2), "3" | "NUM3" => Some(Key::Num3),
            "4" | "NUM4" => Some(Key::Num4), "5" | "NUM5" => Some(Key::Num5),
            "6" | "NUM6" => Some(Key::Num6), "7" | "NUM7" => Some(Key::Num7),
            "8" | "NUM8" => Some(Key::Num8), "9" | "NUM9" => Some(Key::Num9),
            "SPACE" => Some(Key::Space),
            "ENTER" | "RETURN" => Some(Key::Enter),
            "ESC" | "ESCAPE" => Some(Key::Escape),
            "TAB" => Some(Key::Tab),
            "BACKSPACE" => Some(Key::Backspace),
            "SHIFT" | "LSHIFT" => Some(Key::LShift),
            "RSHIFT" => Some(Key::RShift),
            "CTRL" | "LCTRL" => Some(Key::LCtrl),
            "RCTRL" => Some(Key::RCtrl),
            "ALT" | "LALT" => Some(Key::LAlt),
            "UP" => Some(Key::Up),
            "DOWN" => Some(Key::Down),
            "LEFT" => Some(Key::Left),
            "RIGHT" => Some(Key::Right),
            "F1" => Some(Key::F1), "F2" => Some(Key::F2), "F3" => Some(Key::F3),
            "F4" => Some(Key::F4), "F5" => Some(Key::F5), "F6" => Some(Key::F6),
            "F7" => Some(Key::F7), "F8" => Some(Key::F8), "F9" => Some(Key::F9),
            "F10" => Some(Key::F10), "F11" => Some(Key::F11), "F12" => Some(Key::F12),
            _ => None,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MouseButton {
    Left = 0,
    Middle = 1,
    Right = 2,
}

#[derive(Clone, Debug)]
pub struct InputState {
    keys_down: [bool; Key::COUNT],
    keys_prev: [bool; Key::COUNT],
    synth_timers: [f32; Key::COUNT],
    mouse_down: [bool; 3],
    mouse_prev: [bool; 3],
    pub mouse_pos: Vec2,
    pub mouse_delta: Vec2,
    pub scroll_delta: f32,
    pub quit_requested: bool,
    pub focused: bool,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            keys_down: [false; Key::COUNT],
            keys_prev: [false; Key::COUNT],
            synth_timers: [0.0; Key::COUNT],
            mouse_down: [false; 3],
            mouse_prev: [false; 3],
            mouse_pos: Vec2::ZERO,
            mouse_delta: Vec2::ZERO,
            scroll_delta: 0.0,
            quit_requested: false,
            focused: true,
        }
    }
}

impl InputState {
    pub fn begin_frame(&mut self, dt: f32) {
        self.keys_prev = self.keys_down;
        self.mouse_prev = self.mouse_down;
        self.mouse_delta = Vec2::ZERO;
        self.scroll_delta = 0.0;

        for i in 0..Key::COUNT {
            if self.synth_timers[i] > 0.0 {
                self.synth_timers[i] -= dt;
                if self.synth_timers[i] <= 0.0 {
                    self.synth_timers[i] = 0.0;
                    self.keys_down[i] = false;
                } else {
                    self.keys_down[i] = true;
                }
            }
        }
    }

    pub fn set_key(&mut self, key: Key, down: bool) {
        self.keys_down[key as usize] = down;
    }

    pub fn inject_key(&mut self, key: Key, duration_secs: f32) {
        let idx = key as usize;
        self.keys_down[idx] = true;
        self.synth_timers[idx] = duration_secs.max(1e-4);
    }

    pub fn set_mouse_button(&mut self, btn: MouseButton, down: bool) {
        self.mouse_down[btn as usize] = down;
    }

    pub fn clear_all(&mut self) {
        self.keys_down = [false; Key::COUNT];
        self.mouse_down = [false; 3];
    }

    #[inline]
    pub fn key_down(&self, key: Key) -> bool {
        self.keys_down[key as usize]
    }

    #[inline]
    pub fn key_pressed(&self, key: Key) -> bool {
        let i = key as usize;
        self.keys_down[i] && !self.keys_prev[i]
    }

    #[inline]
    pub fn key_released(&self, key: Key) -> bool {
        let i = key as usize;
        !self.keys_down[i] && self.keys_prev[i]
    }

    #[inline]
    pub fn mouse_down(&self, btn: MouseButton) -> bool {
        self.mouse_down[btn as usize]
    }

    #[inline]
    pub fn mouse_pressed(&self, btn: MouseButton) -> bool {
        let i = btn as usize;
        self.mouse_down[i] && !self.mouse_prev[i]
    }

    #[inline]
    pub fn mouse_released(&self, btn: MouseButton) -> bool {
        let i = btn as usize;
        !self.mouse_down[i] && self.mouse_prev[i]
    }
}

// ---------------------------------------------------------------------------
// OpenGL 3.3 Core constants & function table
// ---------------------------------------------------------------------------

pub type GLenum = c_uint;
pub type GLboolean = c_uchar;
pub type GLbitfield = c_uint;
pub type GLint = c_int;
pub type GLsizei = c_int;
pub type GLuint = c_uint;
pub type GLfloat = c_float;
pub type GLsizeiptr = isize;
pub type GLintptr = isize;

pub const GL_FALSE: GLboolean = 0;
pub const GL_TRUE: GLboolean = 1;

pub const GL_DEPTH_BUFFER_BIT: GLbitfield = 0x0000_0100;
pub const GL_COLOR_BUFFER_BIT: GLbitfield = 0x0000_4000;

pub const GL_LINES: GLenum = 0x0001;
pub const GL_TRIANGLES: GLenum = 0x0004;

pub const GL_LESS: GLenum = 0x0201;
pub const GL_LEQUAL: GLenum = 0x0203;

pub const GL_SRC_ALPHA: GLenum = 0x0302;
pub const GL_ONE_MINUS_SRC_ALPHA: GLenum = 0x0303;

pub const GL_NONE: GLenum = 0;
pub const GL_FRONT: GLenum = 0x0404;
pub const GL_BACK: GLenum = 0x0405;
pub const GL_FRONT_AND_BACK: GLenum = 0x0408;
pub const GL_CCW: GLenum = 0x0901;

pub const GL_CULL_FACE: GLenum = 0x0B44;
pub const GL_DEPTH_TEST: GLenum = 0x0B71;
pub const GL_BLEND: GLenum = 0x0BE2;

pub const GL_PACK_ALIGNMENT: GLenum = 0x0D05;
pub const GL_UNPACK_ALIGNMENT: GLenum = 0x0CF5;

pub const GL_TEXTURE_2D: GLenum = 0x0DE1;
pub const GL_UNSIGNED_BYTE: GLenum = 0x1401;
pub const GL_UNSIGNED_INT: GLenum = 0x1405;
pub const GL_FLOAT: GLenum = 0x1406;

pub const GL_LINE: GLenum = 0x1B01;
pub const GL_FILL: GLenum = 0x1B02;

pub const GL_RGBA: GLenum = 0x1908;
pub const GL_DEPTH_COMPONENT: GLenum = 0x1902;
pub const GL_RGBA8: GLenum = 0x8058;
pub const GL_DEPTH_COMPONENT24: GLenum = 0x81A6;

pub const GL_NEAREST: GLint = 0x2600;
pub const GL_LINEAR: GLint = 0x2601;
pub const GL_TEXTURE_MAG_FILTER: GLenum = 0x2800;
pub const GL_TEXTURE_MIN_FILTER: GLenum = 0x2801;
pub const GL_TEXTURE_WRAP_S: GLenum = 0x2802;
pub const GL_TEXTURE_WRAP_T: GLenum = 0x2803;
pub const GL_CLAMP_TO_EDGE: GLint = 0x812F;
pub const GL_REPEAT: GLint = 0x2901;

pub const GL_VENDOR: GLenum = 0x1F00;
pub const GL_RENDERER: GLenum = 0x1F01;
pub const GL_VERSION: GLenum = 0x1F02;

pub const GL_TEXTURE0: GLenum = 0x84C0;
pub const GL_TEXTURE1: GLenum = 0x84C1;
pub const GL_TEXTURE2: GLenum = 0x84C2;
pub const GL_TEXTURE3: GLenum = 0x84C3;
pub const GL_TEXTURE4: GLenum = 0x84C4;

pub const GL_ARRAY_BUFFER: GLenum = 0x8892;
pub const GL_ELEMENT_ARRAY_BUFFER: GLenum = 0x8893;
pub const GL_STATIC_DRAW: GLenum = 0x88E4;
pub const GL_DYNAMIC_DRAW: GLenum = 0x88E8;

pub const GL_FRAGMENT_SHADER: GLenum = 0x8B30;
pub const GL_VERTEX_SHADER: GLenum = 0x8B31;
pub const GL_COMPILE_STATUS: GLenum = 0x8B81;
pub const GL_LINK_STATUS: GLenum = 0x8B82;
pub const GL_INFO_LOG_LENGTH: GLenum = 0x8B84;

pub const GL_FRAMEBUFFER: GLenum = 0x8D40;
pub const GL_RENDERBUFFER: GLenum = 0x8D41;
pub const GL_COLOR_ATTACHMENT0: GLenum = 0x8CE0;
pub const GL_DEPTH_ATTACHMENT: GLenum = 0x8D00;
pub const GL_FRAMEBUFFER_COMPLETE: GLenum = 0x8CD5;

/// Loaded OpenGL 3.3 Core function pointers.
#[allow(non_snake_case)]
pub struct GlFns {
    pub glGetError: unsafe extern "system" fn() -> GLenum,
    pub glGetString: unsafe extern "system" fn(GLenum) -> *const c_uchar,
    pub glViewport: unsafe extern "system" fn(GLint, GLint, GLsizei, GLsizei),
    pub glClearColor: unsafe extern "system" fn(GLfloat, GLfloat, GLfloat, GLfloat),
    pub glClear: unsafe extern "system" fn(GLbitfield),
    pub glEnable: unsafe extern "system" fn(GLenum),
    pub glDisable: unsafe extern "system" fn(GLenum),
    pub glDepthFunc: unsafe extern "system" fn(GLenum),
    pub glDepthMask: unsafe extern "system" fn(GLboolean),
    pub glColorMask: unsafe extern "system" fn(GLboolean, GLboolean, GLboolean, GLboolean),
    pub glCullFace: unsafe extern "system" fn(GLenum),
    pub glFrontFace: unsafe extern "system" fn(GLenum),
    pub glBlendFunc: unsafe extern "system" fn(GLenum, GLenum),
    pub glPolygonMode: unsafe extern "system" fn(GLenum, GLenum),
    pub glLineWidth: unsafe extern "system" fn(GLfloat),
    pub glPixelStorei: unsafe extern "system" fn(GLenum, GLint),
    pub glReadPixels: unsafe extern "system" fn(GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, *mut c_void),
    pub glFinish: unsafe extern "system" fn(),

    pub glGenVertexArrays: unsafe extern "system" fn(GLsizei, *mut GLuint),
    pub glBindVertexArray: unsafe extern "system" fn(GLuint),
    pub glDeleteVertexArrays: unsafe extern "system" fn(GLsizei, *const GLuint),

    pub glGenBuffers: unsafe extern "system" fn(GLsizei, *mut GLuint),
    pub glBindBuffer: unsafe extern "system" fn(GLenum, GLuint),
    pub glBufferData: unsafe extern "system" fn(GLenum, GLsizeiptr, *const c_void, GLenum),
    pub glBufferSubData: unsafe extern "system" fn(GLenum, GLintptr, GLsizeiptr, *const c_void),
    pub glDeleteBuffers: unsafe extern "system" fn(GLsizei, *const GLuint),

    pub glEnableVertexAttribArray: unsafe extern "system" fn(GLuint),
    pub glVertexAttribPointer: unsafe extern "system" fn(GLuint, GLint, GLenum, GLboolean, GLsizei, *const c_void),

    pub glCreateShader: unsafe extern "system" fn(GLenum) -> GLuint,
    pub glShaderSource: unsafe extern "system" fn(GLuint, GLsizei, *const *const c_char, *const GLint),
    pub glCompileShader: unsafe extern "system" fn(GLuint),
    pub glGetShaderiv: unsafe extern "system" fn(GLuint, GLenum, *mut GLint),
    pub glGetShaderInfoLog: unsafe extern "system" fn(GLuint, GLsizei, *mut GLsizei, *mut c_char),
    pub glDeleteShader: unsafe extern "system" fn(GLuint),

    pub glCreateProgram: unsafe extern "system" fn() -> GLuint,
    pub glAttachShader: unsafe extern "system" fn(GLuint, GLuint),
    pub glLinkProgram: unsafe extern "system" fn(GLuint),
    pub glGetProgramiv: unsafe extern "system" fn(GLuint, GLenum, *mut GLint),
    pub glGetProgramInfoLog: unsafe extern "system" fn(GLuint, GLsizei, *mut GLsizei, *mut c_char),
    pub glUseProgram: unsafe extern "system" fn(GLuint),
    pub glDeleteProgram: unsafe extern "system" fn(GLuint),

    pub glGetUniformLocation: unsafe extern "system" fn(GLuint, *const c_char) -> GLint,
    pub glUniform1i: unsafe extern "system" fn(GLint, GLint),
    pub glUniform1f: unsafe extern "system" fn(GLint, GLfloat),
    pub glUniform2f: unsafe extern "system" fn(GLint, GLfloat, GLfloat),
    pub glUniform3f: unsafe extern "system" fn(GLint, GLfloat, GLfloat, GLfloat),
    pub glUniform4f: unsafe extern "system" fn(GLint, GLfloat, GLfloat, GLfloat, GLfloat),
    pub glUniformMatrix4fv: unsafe extern "system" fn(GLint, GLsizei, GLboolean, *const GLfloat),

    pub glDrawArrays: unsafe extern "system" fn(GLenum, GLint, GLsizei),
    pub glDrawElements: unsafe extern "system" fn(GLenum, GLsizei, GLenum, *const c_void),

    pub glGenTextures: unsafe extern "system" fn(GLsizei, *mut GLuint),
    pub glBindTexture: unsafe extern "system" fn(GLenum, GLuint),
    pub glDeleteTextures: unsafe extern "system" fn(GLsizei, *const GLuint),
    pub glTexImage2D: unsafe extern "system" fn(GLenum, GLint, GLint, GLsizei, GLsizei, GLint, GLenum, GLenum, *const c_void),
    pub glTexParameteri: unsafe extern "system" fn(GLenum, GLenum, GLint),
    pub glActiveTexture: unsafe extern "system" fn(GLenum),

    pub glGenFramebuffers: unsafe extern "system" fn(GLsizei, *mut GLuint),
    pub glBindFramebuffer: unsafe extern "system" fn(GLenum, GLuint),
    pub glDeleteFramebuffers: unsafe extern "system" fn(GLsizei, *const GLuint),
    pub glFramebufferTexture2D: unsafe extern "system" fn(GLenum, GLenum, GLenum, GLuint, GLint),
    pub glFramebufferRenderbuffer: unsafe extern "system" fn(GLenum, GLenum, GLenum, GLuint),
    pub glCheckFramebufferStatus: unsafe extern "system" fn(GLenum) -> GLenum,
    pub glDrawBuffer: unsafe extern "system" fn(GLenum),
    pub glReadBuffer: unsafe extern "system" fn(GLenum),

    pub glGenRenderbuffers: unsafe extern "system" fn(GLsizei, *mut GLuint),
    pub glBindRenderbuffer: unsafe extern "system" fn(GLenum, GLuint),
    pub glDeleteRenderbuffers: unsafe extern "system" fn(GLsizei, *const GLuint),
    pub glRenderbufferStorage: unsafe extern "system" fn(GLenum, GLenum, GLsizei, GLsizei),
}

impl GlFns {
    pub fn load<F: Fn(&str) -> *const c_void>(get_proc: F) -> Result<Self, String> {
        macro_rules! load_fn {
            ($name:ident) => {{
                let ptr = get_proc(stringify!($name));
                if ptr.is_null() {
                    return Err(format!("missing OpenGL symbol: {}", stringify!($name)));
                }
                unsafe { std::mem::transmute(ptr) }
            }};
        }

        Ok(Self {
            glGetError: load_fn!(glGetError),
            glGetString: load_fn!(glGetString),
            glViewport: load_fn!(glViewport),
            glClearColor: load_fn!(glClearColor),
            glClear: load_fn!(glClear),
            glEnable: load_fn!(glEnable),
            glDisable: load_fn!(glDisable),
            glDepthFunc: load_fn!(glDepthFunc),
            glDepthMask: load_fn!(glDepthMask),
            glColorMask: load_fn!(glColorMask),
            glCullFace: load_fn!(glCullFace),
            glFrontFace: load_fn!(glFrontFace),
            glBlendFunc: load_fn!(glBlendFunc),
            glPolygonMode: load_fn!(glPolygonMode),
            glLineWidth: load_fn!(glLineWidth),
            glPixelStorei: load_fn!(glPixelStorei),
            glReadPixels: load_fn!(glReadPixels),
            glFinish: load_fn!(glFinish),

            glGenVertexArrays: load_fn!(glGenVertexArrays),
            glBindVertexArray: load_fn!(glBindVertexArray),
            glDeleteVertexArrays: load_fn!(glDeleteVertexArrays),

            glGenBuffers: load_fn!(glGenBuffers),
            glBindBuffer: load_fn!(glBindBuffer),
            glBufferData: load_fn!(glBufferData),
            glBufferSubData: load_fn!(glBufferSubData),
            glDeleteBuffers: load_fn!(glDeleteBuffers),

            glEnableVertexAttribArray: load_fn!(glEnableVertexAttribArray),
            glVertexAttribPointer: load_fn!(glVertexAttribPointer),

            glCreateShader: load_fn!(glCreateShader),
            glShaderSource: load_fn!(glShaderSource),
            glCompileShader: load_fn!(glCompileShader),
            glGetShaderiv: load_fn!(glGetShaderiv),
            glGetShaderInfoLog: load_fn!(glGetShaderInfoLog),
            glDeleteShader: load_fn!(glDeleteShader),

            glCreateProgram: load_fn!(glCreateProgram),
            glAttachShader: load_fn!(glAttachShader),
            glLinkProgram: load_fn!(glLinkProgram),
            glGetProgramiv: load_fn!(glGetProgramiv),
            glGetProgramInfoLog: load_fn!(glGetProgramInfoLog),
            glUseProgram: load_fn!(glUseProgram),
            glDeleteProgram: load_fn!(glDeleteProgram),

            glGetUniformLocation: load_fn!(glGetUniformLocation),
            glUniform1i: load_fn!(glUniform1i),
            glUniform1f: load_fn!(glUniform1f),
            glUniform2f: load_fn!(glUniform2f),
            glUniform3f: load_fn!(glUniform3f),
            glUniform4f: load_fn!(glUniform4f),
            glUniformMatrix4fv: load_fn!(glUniformMatrix4fv),

            glDrawArrays: load_fn!(glDrawArrays),
            glDrawElements: load_fn!(glDrawElements),

            glGenTextures: load_fn!(glGenTextures),
            glBindTexture: load_fn!(glBindTexture),
            glDeleteTextures: load_fn!(glDeleteTextures),
            glTexImage2D: load_fn!(glTexImage2D),
            glTexParameteri: load_fn!(glTexParameteri),
            glActiveTexture: load_fn!(glActiveTexture),

            glGenFramebuffers: load_fn!(glGenFramebuffers),
            glBindFramebuffer: load_fn!(glBindFramebuffer),
            glDeleteFramebuffers: load_fn!(glDeleteFramebuffers),
            glFramebufferTexture2D: load_fn!(glFramebufferTexture2D),
            glFramebufferRenderbuffer: load_fn!(glFramebufferRenderbuffer),
            glCheckFramebufferStatus: load_fn!(glCheckFramebufferStatus),
            glDrawBuffer: load_fn!(glDrawBuffer),
            glReadBuffer: load_fn!(glReadBuffer),

            glGenRenderbuffers: load_fn!(glGenRenderbuffers),
            glBindRenderbuffer: load_fn!(glBindRenderbuffer),
            glDeleteRenderbuffers: load_fn!(glDeleteRenderbuffers),
            glRenderbufferStorage: load_fn!(glRenderbufferStorage),
        })
    }

    pub fn gl_string(&self, name: GLenum) -> String {
        unsafe {
            let p = (self.glGetString)(name);
            if p.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(p as *const c_char)
                    .to_string_lossy()
                    .into_owned()
            }
        }
    }
}
