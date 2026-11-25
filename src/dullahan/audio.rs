//! Zero-dependency stereo audio mixer (`48 kHz`, `f32` internal -> `i16` ALSA output).
//!
//! - Dynamically loads `libasound.so.2` via `dlopen`; automatically falls back to silent
//!   null mode when `--headless` is enabled or no audio device is present.
//! - Mixing DSP (`MixerState`) is pure Rust and fully unit-tested offline.

use crate::dullahan::formats::wav::{AudioClip, TARGET_SAMPLE_RATE};
#[cfg(target_os = "linux")]
use std::ffi::{CString, c_char, c_int, c_long, c_uint, c_void};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

pub const MAX_VOICES: usize = 32;
const CHUNK_FRAMES: usize = 512;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AudioBus {
    Sfx,
    Music,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct VoiceId(pub u32);

#[derive(Copy, Clone, Debug)]
pub struct PlayParams {
    pub gain: f32,
    pub pitch: f32,
    /// Stereo pan in `[-1.0 (left), 1.0 (right)]`.
    pub pan: f32,
    pub bus: AudioBus,
    pub looping: bool,
}

impl Default for PlayParams {
    fn default() -> Self {
        Self {
            gain: 1.0,
            pitch: 1.0,
            pan: 0.0,
            bus: AudioBus::Sfx,
            looping: false,
        }
    }
}

enum AudioCmd {
    Play { id: VoiceId, clip: Arc<AudioClip>, params: PlayParams },
    UpdateVoice { id: VoiceId, gain: f32, pitch: f32, pan: f32 },
    StopVoice { id: VoiceId },
    StopAll,
    SetBusGain { bus: AudioBus, gain: f32 },
    SetMuted(bool),
}

// ---------------------------------------------------------------------------
// Pure-Rust Mixer DSP (testable offline without speakers)
// ---------------------------------------------------------------------------

struct Voice {
    id: VoiceId,
    clip: Arc<AudioClip>,
    pos: f64,
    gain: f32,
    pitch: f32,
    pan: f32,
    bus: AudioBus,
    looping: bool,
}

pub struct MixerState {
    voices: Vec<Voice>,
    sfx_gain: f32,
    music_gain: f32,
    muted: bool,
}

impl Default for MixerState {
    fn default() -> Self {
        Self {
            voices: Vec::with_capacity(MAX_VOICES),
            sfx_gain: 1.0,
            music_gain: 0.8,
            muted: false,
        }
    }
}

impl MixerState {
    fn apply(&mut self, cmd: AudioCmd) {
        match cmd {
            AudioCmd::Play { id, clip, params } => {
                if clip.frames.is_empty() {
                    return;
                }
                if self.voices.len() >= MAX_VOICES {
                    // Evict oldest non-looping voice, else oldest voice.
                    if let Some(i) = self.voices.iter().position(|v| !v.looping) {
                        self.voices.remove(i);
                    } else {
                        self.voices.remove(0);
                    }
                }
                self.voices.push(Voice {
                    id,
                    clip,
                    pos: 0.0,
                    gain: params.gain.max(0.0),
                    pitch: params.pitch.clamp(0.1, 4.0),
                    pan: params.pan.clamp(-1.0, 1.0),
                    bus: params.bus,
                    looping: params.looping,
                });
            }
            AudioCmd::UpdateVoice { id, gain, pitch, pan } => {
                if let Some(v) = self.voices.iter_mut().find(|v| v.id == id) {
                    v.gain = gain.max(0.0);
                    v.pitch = pitch.clamp(0.1, 4.0);
                    v.pan = pan.clamp(-1.0, 1.0);
                }
            }
            AudioCmd::StopVoice { id } => {
                self.voices.retain(|v| v.id != id);
            }
            AudioCmd::StopAll => {
                self.voices.clear();
            }
            AudioCmd::SetBusGain { bus, gain } => {
                let g = gain.clamp(0.0, 2.0);
                match bus {
                    AudioBus::Sfx => self.sfx_gain = g,
                    AudioBus::Music => self.music_gain = g,
                }
            }
            AudioCmd::SetMuted(m) => {
                self.muted = m;
            }
        }
    }

    /// Mixes `out.len()` stereo frames into `out` (overwriting contents).
    pub fn mix_chunk(&mut self, out: &mut [[f32; 2]]) {
        out.fill([0.0, 0.0]);
        if self.muted || self.voices.is_empty() {
            // Advance non-looping voices even while muted so one-shots expire naturally.
            let step_frames = out.len() as f64;
            self.voices.retain_mut(|v| {
                let len = v.clip.frames.len() as f64;
                v.pos += step_frames * v.pitch as f64;
                if v.looping {
                    v.pos %= len;
                    true
                } else {
                    v.pos < len
                }
            });
            return;
        }

        let sfx_g = self.sfx_gain;
        let mus_g = self.music_gain;

        self.voices.retain_mut(|v| {
            let frames = &v.clip.frames;
            let len = frames.len();
            if len == 0 {
                return false;
            }
            let bus_g = match v.bus {
                AudioBus::Sfx => sfx_g,
                AudioBus::Music => mus_g,
            };
            // Equal-power stereo pan law
            let angle = (v.pan + 1.0) * 0.25 * std::f32::consts::PI;
            let gl = angle.cos() * std::f32::consts::SQRT_2 * v.gain * bus_g;
            let gr = angle.sin() * std::f32::consts::SQRT_2 * v.gain * bus_g;
            let step = v.pitch as f64;

            for slot in out.iter_mut() {
                let i0 = v.pos as usize;
                if i0 >= len {
                    if v.looping {
                        v.pos %= len as f64;
                    } else {
                        return false;
                    }
                }
                let idx = (v.pos as usize) % len;
                let next = if idx + 1 < len {
                    idx + 1
                } else if v.looping {
                    0
                } else {
                    idx
                };
                let frac = (v.pos - v.pos.floor()) as f32;
                let a = frames[idx];
                let b = frames[next];
                let l = a[0] + (b[0] - a[0]) * frac;
                let r = a[1] + (b[1] - a[1]) * frac;
                slot[0] += l * gl;
                slot[1] += r * gr;

                v.pos += step;
                if v.pos >= len as f64 {
                    if v.looping {
                        v.pos -= len as f64;
                    } else {
                        return false;
                    }
                }
            }
            true
        });

        // Soft limiter to prevent harsh digital clipping when many SFX overlap.
        for s in out.iter_mut() {
            s[0] = soft_clip(s[0]);
            s[1] = soft_clip(s[1]);
        }
    }

    pub fn active_voices(&self) -> usize {
        self.voices.len()
    }
}

#[inline]
fn soft_clip(x: f32) -> f32 {
    if x.abs() <= 0.9 {
        x
    } else {
        x.signum() * (0.9 + 0.1 * ((x.abs() - 0.9) * 10.0).tanh())
    }
}

// ---------------------------------------------------------------------------
// Public Audio Engine (ALSA backend + automatic silent null fallback)
// ---------------------------------------------------------------------------

pub struct AudioEngine {
    tx: Option<Sender<AudioCmd>>,
    stop_flag: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    next_id: u32,
    rng_state: u64,
    pub backend_name: &'static str,
}

impl AudioEngine {
    pub fn new(headless: bool) -> Self {
        let stop_flag = Arc::new(AtomicBool::new(false));
        if headless {
            return Self {
                tx: None,
                stop_flag,
                worker: None,
                next_id: 1,
                rng_state: 0x9E37_79B9_7F4A_7C15,
                backend_name: "null(headless)",
            };
        }

        match AudioOutDevice::open() {
            Ok((dev, name)) => {
                let (tx, rx) = mpsc::channel();
                let stop_clone = stop_flag.clone();
                let handle = thread::spawn(move || audio_thread_main(dev, rx, stop_clone));
                Self {
                    tx: Some(tx),
                    stop_flag,
                    worker: Some(handle),
                    next_id: 1,
                    rng_state: 0x9E37_79B9_7F4A_7C15,
                    backend_name: name,
                }
            }
            Err(e) => {
                eprintln!("[audio] backend unavailable ({e}); running in silent fallback mode");
                Self {
                    tx: None,
                    stop_flag,
                    worker: None,
                    next_id: 1,
                    rng_state: 0x9E37_79B9_7F4A_7C15,
                    backend_name: "null(fallback)",
                }
            }
        }
    }

    pub fn play(&mut self, clip: Arc<AudioClip>, params: PlayParams) -> VoiceId {
        let id = VoiceId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        if let Some(tx) = &self.tx {
            let _ = tx.send(AudioCmd::Play { id, clip, params });
        }
        id
    }

    pub fn play_loop(&mut self, clip: Arc<AudioClip>, gain: f32, bus: AudioBus) -> VoiceId {
        self.play(
            clip,
            PlayParams { gain, pitch: 1.0, pan: 0.0, bus, looping: true },
        )
    }

    /// Picks a clip from `variants` and applies subtle `±pitch_jitter` (e.g. `0.06`).
    pub fn play_variant(
        &mut self,
        variants: &[Arc<AudioClip>],
        mut params: PlayParams,
        pitch_jitter: f32,
    ) -> Option<VoiceId> {
        if variants.is_empty() {
            return None;
        }
        let idx = (self.next_rand_u64() as usize) % variants.len();
        if pitch_jitter > 0.0 {
            let r = (self.next_rand_u64() as f64 / u64::MAX as f64) as f32 * 2.0 - 1.0;
            params.pitch *= 1.0 + r * pitch_jitter;
        }
        Some(self.play(variants[idx].clone(), params))
    }

    pub fn set_voice(&self, id: VoiceId, gain: f32, pitch: f32, pan: f32) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(AudioCmd::UpdateVoice { id, gain, pitch, pan });
        }
    }

    pub fn stop_voice(&self, id: VoiceId) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(AudioCmd::StopVoice { id });
        }
    }

    pub fn stop_all(&self) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(AudioCmd::StopAll);
        }
    }

    pub fn set_bus_gain(&self, bus: AudioBus, gain: f32) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(AudioCmd::SetBusGain { bus, gain });
        }
    }

    pub fn set_muted(&self, muted: bool) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(AudioCmd::SetMuted(muted));
        }
    }

    fn next_rand_u64(&mut self) -> u64 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng_state = x;
        x
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        self.tx.take();
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

fn audio_thread_main(mut dev: AudioOutDevice, rx: Receiver<AudioCmd>, stop: Arc<AtomicBool>) {
    let mut mixer = MixerState::default();
    let mut mix_buf = [[0.0f32; 2]; CHUNK_FRAMES];
    let mut pcm_i16 = [0i16; CHUNK_FRAMES * 2];

    while !stop.load(Ordering::Relaxed) {
        while let Ok(cmd) = rx.try_recv() {
            mixer.apply(cmd);
        }
        mixer.mix_chunk(&mut mix_buf);
        for i in 0..CHUNK_FRAMES {
            pcm_i16[i * 2] = (mix_buf[i][0].clamp(-1.0, 1.0) * 32767.0) as i16;
            pcm_i16[i * 2 + 1] = (mix_buf[i][1].clamp(-1.0, 1.0) * 32767.0) as i16;
        }
        dev.write_frames(&pcm_i16, CHUNK_FRAMES);
    }
}

// ---------------------------------------------------------------------------
// Minimal ALSA FFI (`libasound.so.2`) via `dlopen` (Linux)
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
}

#[cfg(target_os = "linux")]
const SND_PCM_STREAM_PLAYBACK: c_int = 0;
#[cfg(target_os = "linux")]
const SND_PCM_FORMAT_S16_LE: c_int = 2;
#[cfg(target_os = "linux")]
const SND_PCM_ACCESS_RW_INTERLEAVED: c_int = 3;

#[cfg(target_os = "linux")]
struct AudioOutDevice {
    lib: *mut c_void,
    pcm: *mut c_void,
    snd_pcm_writei: unsafe extern "C" fn(*mut c_void, *const c_void, c_long) -> c_long,
    snd_pcm_recover: unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int,
    snd_pcm_close: unsafe extern "C" fn(*mut c_void) -> c_int,
}

#[cfg(target_os = "linux")]
unsafe impl Send for AudioOutDevice {}

#[cfg(target_os = "linux")]
impl AudioOutDevice {
    fn open() -> Result<(Self, &'static str), String> {
        let lib_name = CString::new("libasound.so.2").unwrap();
        let lib = unsafe { dlopen(lib_name.as_ptr(), 2) };
        if lib.is_null() {
            return Err("libasound.so.2 not found".into());
        }

        macro_rules! load {
            ($name:literal) => {{
                let c = CString::new($name).unwrap();
                let p = unsafe { dlsym(lib, c.as_ptr()) };
                if p.is_null() {
                    unsafe { dlclose(lib) };
                    return Err(format!("missing ALSA symbol {}", $name));
                }
                unsafe { std::mem::transmute_copy(&p) }
            }};
        }

        let snd_pcm_open: unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int, c_int) -> c_int =
            load!("snd_pcm_open");
        let snd_pcm_set_params: unsafe extern "C" fn(*mut c_void, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int =
            load!("snd_pcm_set_params");
        let snd_pcm_writei: unsafe extern "C" fn(*mut c_void, *const c_void, c_long) -> c_long =
            load!("snd_pcm_writei");
        let snd_pcm_recover: unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int =
            load!("snd_pcm_recover");
        let snd_pcm_close: unsafe extern "C" fn(*mut c_void) -> c_int = load!("snd_pcm_close");

        let dev = CString::new("default").unwrap();
        let mut pcm: *mut c_void = std::ptr::null_mut();
        let rc = unsafe { snd_pcm_open(&mut pcm, dev.as_ptr(), SND_PCM_STREAM_PLAYBACK, 0) };
        if rc < 0 || pcm.is_null() {
            unsafe { dlclose(lib) };
            return Err(format!("snd_pcm_open('default') returned {rc}"));
        }

        // 48 kHz, 2ch, S16_LE, ~30ms target latency
        let rc2 = unsafe {
            snd_pcm_set_params(
                pcm,
                SND_PCM_FORMAT_S16_LE,
                SND_PCM_ACCESS_RW_INTERLEAVED,
                2,
                TARGET_SAMPLE_RATE,
                1,
                30_000,
            )
        };
        if rc2 < 0 {
            unsafe {
                snd_pcm_close(pcm);
                dlclose(lib);
            }
            return Err(format!("snd_pcm_set_params returned {rc2}"));
        }

        Ok((
            Self {
                lib,
                pcm,
                snd_pcm_writei,
                snd_pcm_recover,
                snd_pcm_close,
            },
            "alsa",
        ))
    }

    fn write_frames(&mut self, samples: &[i16], frames: usize) {
        let ret = unsafe { (self.snd_pcm_writei)(self.pcm, samples.as_ptr() as *const c_void, frames as c_long) };
        if ret < 0 {
            unsafe {
                (self.snd_pcm_recover)(self.pcm, ret as c_int, 1);
            }
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for AudioOutDevice {
    fn drop(&mut self) {
        unsafe {
            if !self.pcm.is_null() {
                (self.snd_pcm_close)(self.pcm);
            }
            if !self.lib.is_null() {
                dlclose(self.lib);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Minimal Windows Multimedia (`winmm.dll` waveOut) stereo output
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod winmm {
    use super::{CHUNK_FRAMES, TARGET_SAMPLE_RATE};
    use std::ffi::{c_char, c_uint, c_ulong, c_ushort, c_void};
    use std::ptr;
    use std::time::Duration;

    type HWAVEOUT = *mut c_void;
    type MMRESULT = c_uint;
    const WAVE_MAPPER: c_uint = 0xFFFF_FFFF;
    const WAVE_FORMAT_PCM: c_ushort = 1;
    const WHDR_DONE: c_ulong = 0x0000_0001;
    const WHDR_INQUEUE: c_ulong = 0x0000_0010;
    const NUM_BUFS: usize = 4;

    #[repr(C)]
    #[allow(non_snake_case)]
    struct WAVEFORMATEX {
        wFormatTag: c_ushort,
        nChannels: c_ushort,
        nSamplesPerSec: c_ulong,
        nAvgBytesPerSec: c_ulong,
        nBlockAlign: c_ushort,
        wBitsPerSample: c_ushort,
        cbSize: c_ushort,
    }

    #[repr(C)]
    #[allow(non_snake_case)]
    struct WAVEHDR {
        lpData: *mut c_char,
        dwBufferLength: c_ulong,
        dwBytesRecorded: c_ulong,
        dwUser: usize,
        dwFlags: c_ulong,
        dwLoops: c_ulong,
        lpNext: *mut WAVEHDR,
        reserved: usize,
    }

    #[link(name = "winmm")]
    unsafe extern "system" {
        fn waveOutOpen(
            phwo: *mut HWAVEOUT,
            uDeviceID: c_uint,
            pwfx: *const WAVEFORMATEX,
            dwCallback: usize,
            dwInstance: usize,
            fdwOpen: c_ulong,
        ) -> MMRESULT;
        fn waveOutPrepareHeader(hwo: HWAVEOUT, pwh: *mut WAVEHDR, cbwh: c_uint) -> MMRESULT;
        fn waveOutUnprepareHeader(hwo: HWAVEOUT, pwh: *mut WAVEHDR, cbwh: c_uint) -> MMRESULT;
        fn waveOutWrite(hwo: HWAVEOUT, pwh: *mut WAVEHDR, cbwh: c_uint) -> MMRESULT;
        fn waveOutReset(hwo: HWAVEOUT) -> MMRESULT;
        fn waveOutClose(hwo: HWAVEOUT) -> MMRESULT;
    }

    pub struct AudioOutDevice {
        hwo: HWAVEOUT,
        bufs: Box<[[i16; CHUNK_FRAMES * 2]; NUM_BUFS]>,
        hdrs: Box<[WAVEHDR; NUM_BUFS]>,
        cursor: usize,
    }

    unsafe impl Send for AudioOutDevice {}

    impl AudioOutDevice {
        pub fn open() -> Result<(Self, &'static str), String> {
            let wfx = WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_PCM,
                nChannels: 2,
                nSamplesPerSec: TARGET_SAMPLE_RATE as c_ulong,
                nAvgBytesPerSec: (TARGET_SAMPLE_RATE * 4) as c_ulong,
                nBlockAlign: 4,
                wBitsPerSample: 16,
                cbSize: 0,
            };
            let mut hwo: HWAVEOUT = ptr::null_mut();
            let rc = unsafe { waveOutOpen(&mut hwo, WAVE_MAPPER, &wfx, 0, 0, 0) };
            if rc != 0 || hwo.is_null() {
                return Err(format!("waveOutOpen returned {rc}"));
            }

            let mut bufs = Box::new([[0i16; CHUNK_FRAMES * 2]; NUM_BUFS]);
            let mut hdrs = Box::new(std::array::from_fn(|_| WAVEHDR {
                lpData: ptr::null_mut(),
                dwBufferLength: (CHUNK_FRAMES * 4) as c_ulong,
                dwBytesRecorded: 0,
                dwUser: 0,
                dwFlags: WHDR_DONE,
                dwLoops: 0,
                lpNext: ptr::null_mut(),
                reserved: 0,
            }));

            let hdr_sz = std::mem::size_of::<WAVEHDR>() as c_uint;
            for i in 0..NUM_BUFS {
                hdrs[i].lpData = bufs[i].as_mut_ptr() as *mut c_char;
                hdrs[i].dwFlags = 0;
                unsafe {
                    waveOutPrepareHeader(hwo, &mut hdrs[i], hdr_sz);
                }
                hdrs[i].dwFlags |= WHDR_DONE;
            }

            Ok((Self { hwo, bufs, hdrs, cursor: 0 }, "winmm"))
        }

        pub fn write_frames(&mut self, samples: &[i16], frames: usize) {
            let i = self.cursor;
            let hdr_sz = std::mem::size_of::<WAVEHDR>() as c_uint;
            // Wait until this ring buffer slot finishes playback
            let mut spins = 0u32;
            while (unsafe { ptr::read_volatile(&self.hdrs[i].dwFlags) } & WHDR_INQUEUE) != 0 && spins < 200 {
                std::thread::sleep(Duration::from_millis(2));
                spins += 1;
            }
            let n = (frames * 2).min(CHUNK_FRAMES * 2).min(samples.len());
            self.bufs[i][..n].copy_from_slice(&samples[..n]);
            self.hdrs[i].dwBufferLength = (n * 2) as c_ulong;
            unsafe {
                waveOutWrite(self.hwo, &mut self.hdrs[i], hdr_sz);
            }
            self.cursor = (self.cursor + 1) % NUM_BUFS;
        }
    }

    impl Drop for AudioOutDevice {
        fn drop(&mut self) {
            let hdr_sz = std::mem::size_of::<WAVEHDR>() as c_uint;
            unsafe {
                waveOutReset(self.hwo);
                for i in 0..NUM_BUFS {
                    waveOutUnprepareHeader(self.hwo, &mut self.hdrs[i], hdr_sz);
                }
                waveOutClose(self.hwo);
            }
        }
    }
}

#[cfg(target_os = "windows")]
use winmm::AudioOutDevice;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixer_pans_loops_and_expires_oneshots() {
        let clip = Arc::new(AudioClip {
            sample_rate: TARGET_SAMPLE_RATE,
            frames: vec![[0.5, 0.5]; 100],
            duration: 100.0 / TARGET_SAMPLE_RATE as f32,
        });

        let mut mixer = MixerState::default();
        // Hard-left one-shot
        mixer.apply(AudioCmd::Play {
            id: VoiceId(1),
            clip: clip.clone(),
            params: PlayParams { gain: 0.5, pitch: 1.0, pan: -1.0, bus: AudioBus::Sfx, looping: false },
        });
        let mut buf = [[0.0f32; 2]; 64];
        mixer.mix_chunk(&mut buf);
        assert!(buf[0][0] > 0.3, "expected left signal, got {}", buf[0][0]);
        assert!(buf[0][1].abs() < 1e-4, "expected silent right channel, got {}", buf[0][1]);
        assert_eq!(mixer.active_voices(), 1);

        // Second chunk finishes the 100-frame one-shot
        mixer.mix_chunk(&mut buf);
        assert_eq!(mixer.active_voices(), 0);

        // Looping voice survives indefinitely
        mixer.apply(AudioCmd::Play {
            id: VoiceId(2),
            clip,
            params: PlayParams { gain: 0.4, pitch: 2.0, pan: 0.0, bus: AudioBus::Music, looping: true },
        });
        for _ in 0..10 {
            mixer.mix_chunk(&mut buf);
        }
        assert_eq!(mixer.active_voices(), 1);
        assert!(buf[32][0] > 0.1 && buf[32][1] > 0.1);
    }
}
