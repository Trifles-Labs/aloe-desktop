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

    /// Words on the 0–1000 grid for a PNG of known pixel size. The same reader as the live screen.
    #[cfg(test)]
    pub fn ocr_png(png: &[u8], width: u32, height: u32) -> Result<Vec<(String, f64, f64, f64, f64)>, String> {
        let recognized = recognize(png)?;
        Ok(recognized
            .into_iter()
            .map(|(text, x, y, w, h)| (text, to_grid(x as f64, width), to_grid(y as f64, height), to_grid(w as f64, width), to_grid(h as f64, height)))
            .collect())
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

/// The live screen reader, pointed at a PNG instead of the display. Tests use this.
#[cfg(all(test, windows))]
pub fn ocr_png(png: &[u8], width: u32, height: u32) -> Result<Vec<(String, f64, f64, f64, f64)>, String> {
    win::ocr_png(png, width, height)
}

#[cfg(all(test, windows))]
mod accuracy {
    use super::ocr_png;

    const W: u32 = 1280;
    const H: u32 = 720;

    fn loose(value: &str) -> String {
        value.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect()
    }

    fn iou(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> f64 {
        let (ax, ay, aw, ah) = a;
        let (bx, by, bw, bh) = b;
        let x0 = ax.max(bx);
        let y0 = ay.max(by);
        let x1 = (ax + aw).min(bx + bw);
        let y1 = (ay + ah).min(by + bh);
        let inter = (x1 - x0).max(0.0) * (y1 - y0).max(0.0);
        let union = aw * ah + bw * bh - inter;
        if union <= 0.0 { 0.0 } else { inter / union }
    }

    /// A quote Aloe would draw, scored against the rectangle the fixture was painted at.
    #[test]
    fn complex_worksheet_quotes_land_on_the_painted_text() {
        let png = include_bytes!("../tests/fixtures/worksheet.png");
        let words = ocr_png(png, W, H).expect("Windows OCR");
        let heard: Vec<String> = words.iter().map(|(text, ..)| text.clone()).collect();
        assert!(words.len() >= 12, "OCR only saw {heard:?}");

        let page = [
            ("Work by the field", 70.0, 60.0, 304.5, 54.3),
            ("the length of the rod", 70.0, 150.0, 231.6, 102.3),
            ("A. 12 cm", 70.0, 290.0, 124.9, 42.7),
            ("B. 20 cm", 70.0, 340.0, 122.7, 42.7),
            ("C. 7 cm", 70.0, 390.0, 107.8, 42.7),
            ("D. The length of the rod", 70.0, 450.0, 188.1, 84.7),
        ];
        for (phrase, x, y, w, h) in page {
            let truth = (x / W as f64 * 1000.0, y / H as f64 * 1000.0, w / W as f64 * 1000.0, h / H as f64 * 1000.0);
            let hits: Vec<_> = words
                .iter()
                .filter(|(_, wx, wy, ww, wh)| {
                    let word = (*wx, *wy, *ww, *wh);
                    let x0 = wx.max(truth.0);
                    let y0 = wy.max(truth.1);
                    let x1 = (*wx + *ww).min(truth.0 + truth.2);
                    let y1 = (*wy + *wh).min(truth.1 + truth.3);
                    let inter = (x1 - x0).max(0.0) * (y1 - y0).max(0.0);
                    let area = (*ww * *wh).max(1.0);
                    inter / area > 0.4 && iou(word, truth) > 0.0
                })
                .collect();
            let joined: String = hits.iter().map(|(text, ..)| loose(text)).collect();
            assert!(joined.contains(&loose(phrase)), "quote {phrase:?} heard {joined:?} from {heard:?}");
            let (mut x0, mut y0, mut x1, mut y1) = (1000.0_f64, 1000.0_f64, 0.0_f64, 0.0_f64);
            for (_, wx, wy, ww, wh) in &hits {
                x0 = x0.min(*wx);
                y0 = y0.min(*wy);
                x1 = x1.max(*wx + *ww);
                y1 = y1.max(*wy + *wh);
            }
            let found = (x0, y0, x1 - x0, y1 - y0);
            let score = iou(found, truth);
            assert!(score > 0.35, "quote {phrase:?} IoU {score:.2} box {found:?} truth {truth:?}");
        }
    }
}
