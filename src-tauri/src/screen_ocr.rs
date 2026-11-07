//! One capture of the primary display, read with Windows OCR. Word boxes come back on the same
//! 0–1000 grid the marks use, so a highlight can sit on quoted text without a vision model.

use serde_json::Value;
use tauri::AppHandle;

#[cfg(windows)]
mod win {
    use image::imageops::{self, FilterType};
    use serde_json::{json, Value};
    use windows::{
        core::HSTRING,
        Globalization::Language,
        Graphics::Imaging::{BitmapDecoder, BitmapPixelFormat, SoftwareBitmap},
        Media::Ocr::OcrEngine,
        Storage::Streams::{DataWriter, InMemoryRandomAccessStream},
        Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED},
    };
    use xcap::Monitor;

    const GRID: f64 = 1000.0;
    /// Longest side sent to OCR. A 4K frame is slower and the text we mark is still readable here.
    const OCR_MAX_SIDE: u32 = 1920;

    fn to_grid(px: f64, size: u32) -> f64 {
        ((px / size.max(1) as f64) * GRID * 10.0).round() / 10.0
    }

    fn ocr_engine() -> Result<OcrEngine, String> {
        if let Ok(engine) = OcrEngine::TryCreateFromUserProfileLanguages() {
            return Ok(engine);
        }
        let lang = Language::CreateLanguage(&HSTRING::from("en-US")).map_err(|e| e.to_string())?;
        OcrEngine::TryCreateFromLanguage(&lang).map_err(|e| {
            format!("Windows OCR has no language pack ({e}). Install English (OCR) in Windows language settings.")
        })
    }

    fn recognize(png: &[u8]) -> Result<Vec<(String, f32, f32, f32, f32)>, String> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let stream = InMemoryRandomAccessStream::new().map_err(|e| e.to_string())?;
        let writer = DataWriter::CreateDataWriter(&stream).map_err(|e| e.to_string())?;
        writer.WriteBytes(png).map_err(|e| e.to_string())?;
        writer.StoreAsync().map_err(|e| e.to_string())?.get().map_err(|e| e.to_string())?;
        writer.DetachStream().map_err(|e| e.to_string())?;
        stream.Seek(0).map_err(|e| e.to_string())?;
        let decoder = BitmapDecoder::CreateAsync(&stream).map_err(|e| e.to_string())?.get().map_err(|e| e.to_string())?;
        let decoded = decoder.GetSoftwareBitmapAsync().map_err(|e| e.to_string())?.get().map_err(|e| e.to_string())?;
        let bitmap = if decoded.BitmapPixelFormat().ok() == Some(BitmapPixelFormat::Bgra8) {
            decoded
        } else {
            SoftwareBitmap::Convert(&decoded, BitmapPixelFormat::Bgra8).map_err(|e| e.to_string())?
        };
        let result = ocr_engine()?.RecognizeAsync(&bitmap).map_err(|e| e.to_string())?.get().map_err(|e| e.to_string())?;
        let mut words = Vec::new();
        for line in result.Lines().map_err(|e| e.to_string())? {
            for word in line.Words().map_err(|e| e.to_string())? {
                let text = word.Text().map(|value| value.to_string()).unwrap_or_default();
                if text.trim().is_empty() {
                    continue;
                }
                let rect = word.BoundingRect().map_err(|e| e.to_string())?;
                if rect.Width < 1.0 || rect.Height < 1.0 {
                    continue;
                }
                words.push((text, rect.X, rect.Y, rect.Width, rect.Height));
            }
        }
        Ok(words)
    }

    pub fn read() -> Result<Value, String> {
        let monitors = Monitor::all().map_err(|e| format!("Could not list displays: {e}"))?;
        let monitor = monitors
            .iter()
            .find(|monitor| monitor.is_primary())
            .or_else(|| monitors.first())
            .ok_or_else(|| "No display found.".to_string())?;
        let captured = monitor.capture_image().map_err(|e| format!("Screen capture failed: {e}"))?;
        let (full_w, full_h) = captured.dimensions();
        let long = full_w.max(full_h);
        let ocr_image = if long > OCR_MAX_SIDE {
            let scale = OCR_MAX_SIDE as f64 / long as f64;
            imageops::resize(
                &captured,
                ((full_w as f64) * scale).round().max(1.0) as u32,
                ((full_h as f64) * scale).round().max(1.0) as u32,
                FilterType::Triangle,
            )
        } else {
            captured
        };
        let (ocr_w, ocr_h) = ocr_image.dimensions();
        let mut png = Vec::new();
        {
            use image::ImageEncoder;
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(ocr_image.as_raw(), ocr_w, ocr_h, image::ExtendedColorType::Rgba8)
                .map_err(|e| format!("Could not encode the screenshot: {e}"))?;
        }
        let recognized = match recognize(&png) {
            Ok(words) => words,
            Err(error) => {
                crate::config::debug_log("voice", "ocr_failed", &error);
                return Ok(json!({ "words": [], "warning": error }));
            }
        };
        let sx = full_w as f64 / ocr_w.max(1) as f64;
        let sy = full_h as f64 / ocr_h.max(1) as f64;
        let words: Vec<Value> = recognized
            .into_iter()
            .map(|(text, x, y, w, h)| {
                json!({
                    "text": text,
                    "x": to_grid(x as f64 * sx, full_w),
                    "y": to_grid(y as f64 * sy, full_h),
                    "w": to_grid(w as f64 * sx, full_w),
                    "h": to_grid(h as f64 * sy, full_h),
                })
            })
            .collect();
        crate::config::debug_log("voice", "ocr", format!("{} words from {full_w}x{full_h}", words.len()));
        Ok(json!({ "words": words }))
    }
}

/// The overlay is hidden for the capture so marks already on screen are not read back as text.
#[cfg(windows)]
pub async fn read_screen_text(app: &AppHandle, _input: &Value) -> Result<Value, String> {
    let _hidden = crate::voice::overlay::own_capture(app).await;
    tauri::async_runtime::spawn_blocking(win::read).await.map_err(|e| e.to_string())?
}

#[cfg(not(windows))]
pub async fn read_screen_text(_app: &AppHandle, _input: &Value) -> Result<Value, String> {
    Err("Reading the screen's text is only available on Windows.".to_string())
}
