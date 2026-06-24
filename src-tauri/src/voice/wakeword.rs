use vosk::{Model, Recognizer};

/// Grammar-constrained vocabulary for the always-on listener. Restricting Vosk to this
/// short list (instead of full open vocabulary) keeps wake-word spotting fast and avoids
/// false triggers on unrelated speech. "[unk]" is the fallback bucket for anything else.
const WAKE_PHRASES: &[&str] = &["hey aloe", "aloe", "[unk]"];

pub fn load_model(model_dir: &std::path::Path) -> Result<Model, String> {
    Model::new(model_dir.to_string_lossy().to_string())
        .ok_or_else(|| format!("Could not load Vosk model at {}", model_dir.display()))
}

pub fn new_recognizer(model: &Model, sample_rate: f32) -> Result<Recognizer, String> {
    Recognizer::new_with_grammar(model, sample_rate, WAKE_PHRASES)
        .ok_or_else(|| "Could not create Vosk recognizer.".to_string())
}

/// Open-vocabulary recognizer used to capture the actual question once the wake word has
/// fired. Unlike the grammar-constrained wake-word recognizer, this transcribes free speech.
pub fn new_open_recognizer(model: &Model, sample_rate: f32) -> Result<Recognizer, String> {
    Recognizer::new(model, sample_rate).ok_or_else(|| "Could not create open-vocabulary recognizer.".to_string())
}

/// True if a finalized recognition result is an actual "aloe" wake phrase rather than
/// silence or the "[unk]" fallback.
pub fn is_wake_phrase(text: &str) -> bool {
    let normalized = text.trim();
    !normalized.is_empty() && normalized != "[unk]" && normalized.contains("aloe")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Feeds a WAV fixture through the same grammar-constrained recognizer the live
    /// listener uses, and returns the finalized transcript.
    fn transcribe_fixture(name: &str) -> Option<String> {
        let model_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/vosk-model-small-en-us-0.15");
        if !model_dir.exists() {
            eprintln!("skipping {name}: run scripts/fetch-voice-assets.ps1 to fetch the Vosk model first");
            return None;
        }

        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
        let mut reader = hound::WavReader::open(&fixture_path)
            .unwrap_or_else(|e| panic!("could not open fixture {}: {e}", fixture_path.display()));
        let sample_rate = reader.spec().sample_rate as f32;
        let samples = reader.samples::<i16>().collect::<hound::Result<Vec<i16>>>().expect("could not read fixture samples");

        let model = load_model(&model_dir).expect("could not load model");
        let mut recognizer = new_recognizer(&model, sample_rate).expect("could not create recognizer");

        for chunk in samples.chunks(4000) {
            let _ = recognizer.accept_waveform(chunk);
        }

        recognizer.final_result().single().map(|result| result.text.to_string())
    }

    #[test]
    fn recognizes_hey_aloe_wake_phrase() {
        let Some(text) = transcribe_fixture("hey-aloe.wav") else { return };
        assert!(is_wake_phrase(&text), "expected a wake phrase, got {text:?}");
    }

    #[test]
    fn ignores_unrelated_speech() {
        let Some(text) = transcribe_fixture("unrelated-speech.wav") else { return };
        assert!(!is_wake_phrase(&text), "expected no wake phrase, got {text:?}");
    }
}
