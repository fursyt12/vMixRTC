//! NDI: приём видео, конвертация кадров и MJPEG-поток для интерфейса.
//!
//! Зачем так: оригинал показывает NDI-монитор WPF-контролом (`CustomUI` провайдера).
//! В порте интерфейс — webview, поэтому кадры отдаются как MJPEG-поток, который
//! подставляется в `<img>`: это дёшево, не требует IPC на каждый кадр и работает
//! в любом браузерном движке.
//!
//! Источник кадров — трейт [`Backend`]:
//! * [`TestBackend`] — синтетическая картинка (тесты и демонстрация без NDI runtime);
//! * [`RuntimeBackend`] — FFI к NDI runtime (`libndi.so` / `Processing.NDI.Lib.x64.dll`).
//!
//! **Честно про FFI:** биндинги написаны по публичным заголовкам NDI 5 и загружаются
//! динамически, поэтому крейт собирается без установленного NDI. Проверить их в бою можно
//! только на машине с NDI runtime — здесь его нет, поэтому FFI покрыт лишь компиляцией и
//! проверкой аккуратного отказа, когда библиотека не найдена.

use anyhow::{anyhow, Context, Result};
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

// --------------------------------------------------------------- кадр

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// Формат NDI по умолчанию: 2 байта на пиксель (Y0 U Y1 V).
    Uyvy,
    /// BGRA/BGRX — то, что отдают многие источники.
    Bgra,
    Rgba,
    /// Упакованный RGB (3 байта) — так отдаёт тестовый сигнал.
    Rgb,
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Байт в строке (может быть больше `width * bytes_per_pixel`).
    pub stride: usize,
    pub format: PixelFormat,
    pub data: Vec<u8>,
}

impl Frame {
    /// Преобразовать кадр в RGB8 (то, что ждёт JPEG-энкодер).
    pub fn to_rgb(&self) -> Vec<u8> {
        let width = self.width as usize;
        let height = self.height as usize;
        let mut rgb = vec![0u8; width * height * 3];
        let bytes_per_pixel = match self.format {
            PixelFormat::Uyvy => 2,
            PixelFormat::Rgb => 3,
            PixelFormat::Bgra | PixelFormat::Rgba => 4,
        };

        for y in 0..height {
            let row = y * self.stride;
            for x in 0..width {
                let index = row + x * bytes_per_pixel;
                let target = (y * width + x) * 3;
                if index + bytes_per_pixel > self.data.len() {
                    continue;
                }
                let (r, g, b) = match self.format {
                    PixelFormat::Uyvy => {
                        // два пикселя в четырёх байтах: Y0 U Y1 V
                        let pair = (x / 2) * 4;
                        let luma_index = row + pair + if x % 2 == 0 { 0 } else { 2 };
                        if luma_index + 3 > self.data.len() {
                            continue;
                        }
                        let luma = self.data[luma_index] as f32;
                        let u = self.data[luma_index + 1] as f32 - 128.0;
                        let v = self.data[luma_index + 3] as f32 - 128.0;
                        yuv_to_rgb(luma, u, v)
                    }
                    PixelFormat::Bgra => (
                        self.data[index + 2],
                        self.data[index + 1],
                        self.data[index],
                    ),
                    PixelFormat::Rgba | PixelFormat::Rgb => (
                        self.data[index],
                        self.data[index + 1],
                        self.data[index + 2],
                    ),
                };
                rgb[target] = r;
                rgb[target + 1] = g;
                rgb[target + 2] = b;
            }
        }
        rgb
    }

    pub fn rgb(&self) -> RgbFrame {
        RgbFrame {
            width: self.width,
            height: self.height,
            data: self.to_rgb(),
        }
    }
}

fn yuv_to_rgb(luma: f32, u: f32, v: f32) -> (u8, u8, u8) {
    let c = luma - 16.0;
    let d = u;
    let e = v;
    let r = (1.164 * c + 1.596 * e).clamp(0.0, 255.0) as u8;
    let g = (1.164 * c - 0.392 * d - 0.813 * e).clamp(0.0, 255.0) as u8;
    let b = (1.164 * c + 2.017 * d).clamp(0.0, 255.0) as u8;
    (r, g, b)
}

#[derive(Debug, Clone)]
pub struct RgbFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl RgbFrame {
    pub fn to_jpeg(&self, quality: u8) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let encoder = jpeg_encoder::Encoder::new(&mut out, quality);
        encoder
            .encode(&self.data, self.width as u16, self.height as u16, jpeg_encoder::ColorType::Rgb)
            .context("кодирование JPEG")?;
        Ok(out)
    }
}

// --------------------------------------------------------------- источники кадров

/// Откуда берутся кадры: NDI runtime или синтетика.
pub trait Backend: Send {
    /// Следующий кадр; `None`, если кадров пока нет.
    fn next_frame(&mut self) -> Option<Frame>;

    /// Что показывать в интерфейсе (имя источника или пометка тестового режима).
    fn describe(&self) -> String;
}

/// Синтетическая картинка: цветные полосы + бегущий маркер, чтобы было видно движение.
pub struct TestBackend {
    width: u32,
    height: u32,
    tick: u64,
    label: String,
}

impl TestBackend {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            tick: 0,
            label: "тестовый сигнал".to_string(),
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }
}

impl Backend for TestBackend {
    fn next_frame(&mut self) -> Option<Frame> {
        self.tick += 1;
        let bars: [[u8; 3]; 8] = [
            [255, 255, 255],
            [255, 255, 0],
            [0, 255, 255],
            [0, 255, 0],
            [255, 0, 255],
            [255, 0, 0],
            [0, 0, 255],
            [16, 16, 16],
        ];
        let width = self.width as usize;
        let height = self.height as usize;
        let mut data = vec![0u8; width * height * 3];
        let bar_width = (width / bars.len()).max(1);
        // шаг 13 и оранжевый маркер: он не сливается с полосами и виден в каждом кадре
        let marker_x = (self.tick as usize * 13) % width;
        for y in 0..height {
            for x in 0..width {
                let index = (y * width + x) * 3;
                let mut color = bars[(x / bar_width).min(bars.len() - 1)];
                // движущийся маркер, чтобы в потоке было видно обновление кадров
                if x.abs_diff(marker_x) < 5 {
                    color = [255, 128, 0];
                }
                if y < height / 12 {
                    color = [0, 0, 0];
                }
                data[index] = color[0];
                data[index + 1] = color[1];
                data[index + 2] = color[2];
            }
        }
        Some(Frame {
            width: self.width,
            height: self.height,
            stride: width * 3,
            format: PixelFormat::Rgb,
            data,
        })
    }

    fn describe(&self) -> String {
        self.label.clone()
    }
}

// --------------------------------------------------------------- NDI runtime (FFI)

mod ffi {
    //! Минимальные биндинги NDI 5 (`NDIlib_*`), загружаемые динамически.

    use anyhow::{anyhow, Context, Result};
    use libloading::Library;
    use std::ffi::c_void;

    pub type FindInstance = *mut c_void;
    pub type RecvInstance = *mut c_void;

    #[repr(C)]
    pub struct NdiSource {
        pub p_ndi_name: *const std::os::raw::c_char,
        pub p_url_address: *const std::os::raw::c_char,
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    pub struct VideoFrameV2 {
        pub xres: i32,
        pub yres: i32,
        pub four_cc: u32,
        pub frame_rate_n: i32,
        pub frame_rate_d: i32,
        pub picture_aspect_ratio: f32,
        pub frame_format_type: i32,
        pub timecode: i64,
        pub p_data: *mut u8,
        pub line_stride_in_bytes: i32,
        pub p_metadata: *const std::os::raw::c_char,
        pub timestamp: i64,
    }

    pub const FRAME_TYPE_VIDEO: i32 = 1;
    pub const FOURCC_UYVY: u32 = u32::from_le_bytes(*b"UYVY");
    pub const FOURCC_BGRA: u32 = u32::from_le_bytes(*b"BGRA");
    pub const FOURCC_BGRX: u32 = u32::from_le_bytes(*b"BGRX");
    pub const FOURCC_RGBA: u32 = u32::from_le_bytes(*b"RGBA");

    pub struct Ndi {
        pub library: Library,
    }

    impl Ndi {
        /// Загрузить NDI runtime, если он есть в системе.
        pub fn open() -> Result<Self> {
            let names: &[&str] = if cfg!(windows) {
                &["Processing.NDI.Lib.x64.dll", "Processing.NDI.Lib.x86.dll"]
            } else if cfg!(target_os = "macos") {
                &["libndi.dylib", "libndi.5.dylib"]
            } else {
                &["libndi.so.5", "libndi.so"]
            };
            for name in names {
                // SAFETY: загрузка сторонней библиотеки — ожидаемое поведение NDI-клиента.
                if let Ok(library) = unsafe { Library::new(*name) } {
                    return Ok(Self { library });
                }
            }
            Err(anyhow!(
                "NDI runtime не найден (искали {:?})",
                names
            ))
        }

        pub fn symbol<T>(&self, name: &str) -> Result<T>
        where
            T: Copy,
        {
            // SAFETY: имена символов соответствуют публичному API NDI.
            unsafe {
                self.library
                    .get(name.as_bytes())
                    .map(|symbol| *symbol)
                    .with_context(|| format!("нет символа {name} в NDI runtime"))
            }
        }
    }
}

/// Источник кадров из NDI runtime.
///
/// `Send` объявляем вручную: внутри сырые указатели NDI, а бэкенд живёт в потоке MJPEG-сервера.
pub struct RuntimeBackend {
    ffi: ffi::Ndi,
    find: ffi::FindInstance,
    recv: ffi::RecvInstance,
    source_name: String,
    initialized: bool,
}

impl RuntimeBackend {
    /// Подключиться к источнику по имени (`None` — первый найденный).
    pub fn connect(source_name: Option<&str>) -> Result<Self> {
        let ffi = ffi::Ndi::open()?;
        let initialize: unsafe extern "C" fn() -> bool = ffi.symbol("NDIlib_initialize")?;
        if !unsafe { initialize() } {
            return Err(anyhow!("NDIlib_initialize вернул false"));
        }
        let find_create: unsafe extern "C" fn(*const std::ffi::c_void) -> ffi::FindInstance =
            ffi.symbol("NDIlib_find_create_v2")?;
        let find_wait: unsafe extern "C" fn(ffi::FindInstance, u32) -> bool =
            ffi.symbol("NDIlib_find_wait_for_sources")?;
        let find_sources: unsafe extern "C" fn(ffi::FindInstance, *mut u32) -> *const ffi::NdiSource =
            ffi.symbol("NDIlib_find_get_current_sources")?;

        let find = unsafe { find_create(std::ptr::null()) };
        if find.is_null() {
            return Err(anyhow!("не удалось создать NDI finder"));
        }
        unsafe { find_wait(find, 5000) };
        let mut count: u32 = 0;
        let sources = unsafe { find_sources(find, &mut count) };
        if sources.is_null() || count == 0 {
            return Err(anyhow!("NDI-источники не найдены"));
        }
        let wanted = source_name.unwrap_or_default();
        let mut chosen: Option<ffi::NdiSource> = None;
        for index in 0..count as usize {
            // SAFETY: массив принадлежит finder'у и валиден до его уничтожения.
            let source = unsafe { &*sources.add(index) };
            let name = unsafe {
                std::ffi::CStr::from_ptr(source.p_ndi_name)
                    .to_string_lossy()
                    .into_owned()
            };
            if wanted.is_empty() || name == wanted {
                chosen = Some(ffi::NdiSource {
                    p_ndi_name: source.p_ndi_name,
                    p_url_address: source.p_url_address,
                });
                break;
            }
        }
        let source = chosen.ok_or_else(|| anyhow!("NDI-источник «{wanted}» не найден"))?;

        let recv_create: unsafe extern "C" fn(*const std::ffi::c_void) -> ffi::RecvInstance =
            ffi.symbol("NDIlib_recv_create_v3")?;
        let recv_connect: unsafe extern "C" fn(ffi::RecvInstance, *const ffi::NdiSource) = ffi
            .symbol("NDIlib_recv_connect")?;
        let recv = unsafe { recv_create(std::ptr::null()) };
        if recv.is_null() {
            return Err(anyhow!("не удалось создать NDI-приёмник"));
        }
        let mut owned = ffi::NdiSource {
            p_ndi_name: std::ptr::null(),
            p_url_address: std::ptr::null(),
        };
        // имена держим в CString, чтобы указатели жили во время connect
        let name = std::ffi::CString::new(
            unsafe { std::ffi::CStr::from_ptr(source.p_ndi_name) }.to_bytes(),
        )
        .unwrap_or_default();
        owned.p_ndi_name = name.as_ptr();
        owned.p_url_address = std::ptr::null();
        unsafe { recv_connect(recv, &owned) };
        drop(name);

        Ok(Self {
            ffi,
            find,
            recv,
            source_name: source_name.unwrap_or("первый найденный").to_string(),
            initialized: true,
        })
    }
}

unsafe impl Send for RuntimeBackend {}

impl Backend for RuntimeBackend {
    fn next_frame(&mut self) -> Option<Frame> {
        let capture: unsafe extern "C" fn(
            ffi::RecvInstance,
            *mut ffi::VideoFrameV2,
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
            u32,
        ) -> i32 = self.ffi.symbol("NDIlib_recv_capture_v3").ok()?;
        let free_video: unsafe extern "C" fn(ffi::RecvInstance, *mut ffi::VideoFrameV2) =
            self.ffi.symbol("NDIlib_recv_free_video_v2").ok()?;

        let mut video = ffi::VideoFrameV2::default();
        let frame_type = unsafe {
            capture(
                self.recv,
                &mut video,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                1000,
            )
        };
        if frame_type != ffi::FRAME_TYPE_VIDEO {
            return None;
        }

        let format = match video.four_cc {
            ffi::FOURCC_UYVY => PixelFormat::Uyvy,
            ffi::FOURCC_BGRA | ffi::FOURCC_BGRX => PixelFormat::Bgra,
            ffi::FOURCC_RGBA => PixelFormat::Rgba,
            // неизвестный формат читаем как RGBA — как и раньше, но явной веткой
            _ => PixelFormat::Rgba,
        };
        let bytes_per_pixel = if format == PixelFormat::Uyvy { 2 } else { 4 };
        let length = (video.line_stride_in_bytes as usize).max(video.xres as usize * bytes_per_pixel)
            * video.yres as usize;
        let data = unsafe { std::slice::from_raw_parts(video.p_data, length).to_vec() };
        let frame = Frame {
            width: video.xres as u32,
            height: video.yres as u32,
            stride: video.line_stride_in_bytes.max(0) as usize,
            format,
            data,
        };
        unsafe { free_video(self.recv, &mut video) };
        Some(frame)
    }

    fn describe(&self) -> String {
        format!("NDI: {}", self.source_name)
    }
}

impl Drop for RuntimeBackend {
    fn drop(&mut self) {
        if let Ok(destroy_recv) = self
            .ffi
            .symbol::<unsafe extern "C" fn(ffi::RecvInstance)>("NDIlib_recv_destroy")
        {
            unsafe { destroy_recv(self.recv) };
        }
        if let Ok(destroy_find) = self
            .ffi
            .symbol::<unsafe extern "C" fn(ffi::FindInstance)>("NDIlib_find_destroy")
        {
            unsafe { destroy_find(self.find) };
        }
        if self.initialized {
            if let Ok(destroy) = self.ffi.symbol::<unsafe extern "C" fn()>("NDIlib_destroy") {
                unsafe { destroy() };
            }
        }
    }
}

// --------------------------------------------------------------- MJPEG-поток

/// MJPEG-сервер: отдаёт кадры бэкенда по `http://127.0.0.1:<порт>/stream.mjpg`.
pub struct MjpegServer {
    port: u16,
    stop: Arc<AtomicBool>,
    frames: Arc<AtomicU64>,
}

impl MjpegServer {
    /// Поднять сервер на свободном порту (кадры кодируются в JPEG на лету).
    pub fn start(mut backend: Box<dyn Backend>, quality: u8, fps: u32) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").context("порт для MJPEG")?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(AtomicU64::new(0));
        let stop_flag = stop.clone();
        let frame_counter = frames.clone();

        std::thread::spawn(move || {
            let mut clients: Vec<TcpStream> = Vec::new();
            let mut last = Instant::now();
            let interval = Duration::from_millis((1000 / fps.max(1)) as u64);
            while !stop_flag.load(Ordering::Relaxed) {
                if let Ok((stream, _)) = listener.accept() {
                    let _ = stream.set_nonblocking(true);
                    // заголовок multipart/x-mixed-replace — то, что понимает <img>
                    let mut stream_clone = stream.try_clone().expect("клон сокета");
                    let header = "HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=frame\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n";
                    if stream_clone.write_all(header.as_bytes()).is_ok() {
                        clients.push(stream);
                    }
                }
                if last.elapsed() >= interval && !clients.is_empty() {
                    last = Instant::now();
                    if let Some(frame) = backend.next_frame() {
                        if let Ok(jpeg) = frame.rgb().to_jpeg(quality) {
                            let part = format!(
                                "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                                jpeg.len()
                            );
                            clients.retain_mut(|client| {
                                client
                                    .write_all(part.as_bytes())
                                    .and_then(|_| client.write_all(&jpeg))
                                    .and_then(|_| client.write_all(b"\r\n"))
                                    .and_then(|_| client.flush())
                                    .is_ok()
                            });
                            frame_counter.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        });

        Ok(Self { port, stop, frames })
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/stream.mjpg", self.port)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn frames(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for MjpegServer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn converts_bgra_to_rgb() {
        let frame = Frame {
            width: 2,
            height: 1,
            stride: 8,
            format: PixelFormat::Bgra,
            data: vec![0, 0, 255, 255, 0, 255, 0, 255], // синий, зелёный (BGRA)
        };
        assert_eq!(frame.to_rgb(), vec![255, 0, 0, 0, 255, 0]);
    }

    #[test]
    fn converts_uyvy_to_rgb() {
        // два пикселя: Y0=235 U=128 Y1=16 V=128 → белый и почти чёрный
        let frame = Frame {
            width: 2,
            height: 1,
            stride: 4,
            format: PixelFormat::Uyvy,
            data: vec![235, 128, 16, 128],
        };
        let rgb = frame.to_rgb();
        assert_eq!(rgb.len(), 6);
        assert!(rgb[0] > 200 && rgb[1] > 200 && rgb[2] > 200, "{rgb:?}");
        assert!(rgb[3] < 40 && rgb[4] < 40 && rgb[5] < 40, "{rgb:?}");
    }

    #[test]
    fn converts_rgb_to_rgb() {
        let frame = Frame {
            width: 2,
            height: 1,
            stride: 6,
            format: PixelFormat::Rgb,
            data: vec![10, 20, 30, 200, 210, 220],
        };
        assert_eq!(frame.to_rgb(), vec![10, 20, 30, 200, 210, 220]);
    }

    #[test]
    fn test_backend_frames_move_and_encode() {
        let mut backend = TestBackend::new(320, 180);
        let first = backend.next_frame().unwrap();
        let second = backend.next_frame().unwrap();
        assert_eq!((first.width, first.height), (320, 180));
        let diff = first
            .data
            .iter()
            .zip(second.data.iter())
            .filter(|(left, right)| left != right)
            .count();
        assert!(diff > 0, "кадры одинаковые (различий: {diff})");

        let jpeg = first.rgb().to_jpeg(70).unwrap();
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "нет маркера начала JPEG");
        assert_eq!(&jpeg[jpeg.len() - 2..], &[0xFF, 0xD9], "нет маркера конца JPEG");
        assert!(jpeg.len() > 1000, "слишком маленький JPEG: {}", jpeg.len());
    }

    #[test]
    fn mjpeg_server_streams_frames_over_http() {
        let server = MjpegServer::start(Box::new(TestBackend::new(160, 90)), 60, 20).unwrap();
        assert!(server.url().starts_with("http://127.0.0.1:"));

        let mut stream = TcpStream::connect(("127.0.0.1", server.port())).unwrap();
        stream
            .write_all(b"GET /stream.mjpg HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        let mut received = Vec::new();
        let mut buffer = [0u8; 4096];
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && received.len() < 20_000 {
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => received.extend_from_slice(&buffer[..read]),
                Err(_) => break,
            }
            if received.windows(2).any(|pair| pair == [0xFF, 0xD8])
                && received.len() > 2000
            {
                break;
            }
        }
        let text = String::from_utf8_lossy(&received);
        assert!(
            text.contains("multipart/x-mixed-replace"),
            "нет multipart-заголовка: {}",
            &text[..text.len().min(200)]
        );
        assert!(text.contains("image/jpeg"), "нет части JPEG");
        assert!(
            received.windows(2).any(|pair| pair == [0xFF, 0xD8]),
            "в потоке нет JPEG"
        );
        server.stop();
    }

    #[test]
    fn runtime_backend_reports_missing_library() {
        // NDI runtime в окружении нет — проверяем аккуратный отказ, а не панику
        match RuntimeBackend::connect(None) {
            Ok(_) => eprintln!("NDI runtime найден — проверьте поток вручную"),
            Err(error) => {
                let text = format!("{error:#}");
                assert!(
                    text.contains("NDI") || text.contains("ndi"),
                    "непонятная ошибка: {text}"
                );
            }
        }
    }
}
