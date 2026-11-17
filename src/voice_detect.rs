//! Is someone singing, or rapping, right now? Asked of the sound classifier
//! built into macOS (SoundAnalysis), which recognises voices in a full mix far
//! better than any measure of the spectrum we tried (see PLAN.md, "Vocals and
//! the lead line").
//!
//! Two answers, because different vocals do different things: **singing** (a
//! melodic voice: the classifier's singing, humming and choir, and speech,
//! which is what sung vocals in a mix often read as) and **rapping**. Singing
//! is counted only as far as the voice isn't rap.
//!
//! The analyser feeds it the mono mix as it goes; it listens in 1.5 s windows,
//! half overlapping, so the answer comes about 1.5 s late and changes every
//! 0.75 s: right for "this is a vocal passage", not for single syllables.
//! Other platforms have no classifier: the voice reads 0 there.

#[cfg(target_os = "macos")]
pub use mac::VoiceDetector;

#[cfg(not(target_os = "macos"))]
pub struct VoiceDetector;

#[cfg(not(target_os = "macos"))]
impl VoiceDetector {
    pub fn new(_sample_rate: f32) -> Option<Self> {
        None
    }
    pub fn push(&mut self, _mono: &[f32]) {}
    pub fn voices(&self) -> (f32, f32) {
        (0.0, 0.0)
    }
}

/// Labels of the built-in classifier that are a melodic voice.
pub const SINGING_LABELS: [&str; 4] = ["singing", "speech", "humming", "choir_singing"];
/// …and rap.
pub const RAP_LABEL: &str = "rapping";

/// (singing, rapping) from the classifier's confidence per label.
pub fn voices(confidence: impl Fn(&str) -> f32) -> (f32, f32) {
    let rap = confidence(RAP_LABEL).clamp(0.0, 1.0);
    let melodic = any_of(SINGING_LABELS.map(&confidence));
    (melodic * (1.0 - rap), rap)
}

/// The chance that any of these is present (the classifier's labels are
/// independent: a voice in a mix often reads partly as speech, partly as singing).
pub fn any_of(confidences: impl IntoIterator<Item = f32>) -> f32 {
    1.0 - confidences.into_iter().fold(1.0, |p, c| p * (1.0 - c.clamp(0.0, 1.0)))
}

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    use objc2::{
        AllocAnyThread, DefinedClass, define_class, msg_send,
        rc::Retained,
        runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject},
    };
    use objc2_avf_audio::{AVAudioFormat, AVAudioPCMBuffer};
    use objc2_core_media::CMTime;
    use objc2_foundation::NSString;
    use objc2_sound_analysis::{
        SNAudioStreamAnalyzer, SNClassificationResult, SNClassifierIdentifierVersion1, SNClassifySoundRequest, SNRequest,
        SNResult, SNResultsObserving,
    };

    use super::voices;

    /// Frames handed to the classifier at a time (~90 ms at 44.1 kHz).
    const CHUNK: u32 = 4096;

    pub struct Ivars {
        /// Latest (singing, rapping), as f32 bits.
        singing: Arc<AtomicU32>,
        rapping: Arc<AtomicU32>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "SynestesiaVoiceObserver"]
        #[ivars = Ivars]
        struct Observer;

        unsafe impl NSObjectProtocol for Observer {}

        unsafe impl SNResultsObserving for Observer {
            #[unsafe(method(request:didProduceResult:))]
            unsafe fn request_did_produce_result(
                &self,
                _request: &ProtocolObject<dyn SNRequest>,
                result: &ProtocolObject<dyn SNResult>,
            ) {
                let any: &AnyObject = result.as_ref();
                let Some(result) = any.downcast_ref::<SNClassificationResult>() else { return };
                let confidence = |label: &str| unsafe {
                    result.classificationForIdentifier(&NSString::from_str(label)).map_or(0.0, |c| c.confidence() as f32)
                };
                let (singing, rapping) = voices(confidence);
                self.ivars().singing.store(singing.to_bits(), Ordering::Relaxed);
                self.ivars().rapping.store(rapping.to_bits(), Ordering::Relaxed);
            }
        }
    );

    impl Observer {
        fn new(singing: Arc<AtomicU32>, rapping: Arc<AtomicU32>) -> Retained<Self> {
            let this = Self::alloc().set_ivars(Ivars { singing, rapping });
            unsafe { msg_send![super(this), init] }
        }
    }

    pub struct VoiceDetector {
        analyzer: Retained<SNAudioStreamAnalyzer>,
        buffer: Retained<AVAudioPCMBuffer>,
        // Kept alive while the analyzer uses them.
        _request: Retained<SNClassifySoundRequest>,
        _observer: Retained<Observer>,
        filled: u32,
        position: i64,
        singing: Arc<AtomicU32>,
        rapping: Arc<AtomicU32>,
    }

    impl VoiceDetector {
        pub fn new(sample_rate: f32) -> Option<Self> {
            unsafe {
                let format = AVAudioFormat::initStandardFormatWithSampleRate_channels(AVAudioFormat::alloc(), sample_rate as f64, 1)?;
                let buffer = AVAudioPCMBuffer::initWithPCMFormat_frameCapacity(AVAudioPCMBuffer::alloc(), &format, CHUNK)?;
                let analyzer = SNAudioStreamAnalyzer::initWithFormat(SNAudioStreamAnalyzer::alloc(), &format);
                let request = SNClassifySoundRequest::initWithClassifierIdentifier_error(
                    SNClassifySoundRequest::alloc(),
                    SNClassifierIdentifierVersion1?,
                )
                .ok()?;
                request.setWindowDuration(CMTime::with_seconds(1.5, 48_000));
                request.setOverlapFactor(0.5);
                let singing = Arc::new(AtomicU32::new(0));
                let rapping = Arc::new(AtomicU32::new(0));
                let observer = Observer::new(singing.clone(), rapping.clone());
                analyzer
                    .addRequest_withObserver_error(
                        ProtocolObject::from_ref(&*request),
                        ProtocolObject::from_ref(&*observer),
                    )
                    .ok()?;
                Some(Self { analyzer, buffer, _request: request, _observer: observer, filled: 0, position: 0, singing, rapping })
            }
        }

        /// Feed mono samples (may block briefly while the classifier catches up).
        pub fn push(&mut self, mono: &[f32]) {
            let mut rest = mono;
            while !rest.is_empty() {
                let room = (CHUNK - self.filled) as usize;
                let n = room.min(rest.len());
                unsafe {
                    let data = (*self.buffer.floatChannelData()).as_ptr();
                    std::ptr::copy_nonoverlapping(rest.as_ptr(), data.add(self.filled as usize), n);
                }
                self.filled += n as u32;
                rest = &rest[n..];
                if self.filled == CHUNK {
                    unsafe {
                        self.buffer.setFrameLength(CHUNK);
                        self.analyzer.analyzeAudioBuffer_atAudioFramePosition(&self.buffer, self.position);
                    }
                    self.position += CHUNK as i64;
                    self.filled = 0;
                }
            }
        }

        /// The latest answer: how likely (singing, rapping) are, 0..1 each.
        pub fn voices(&self) -> (f32, f32) {
            let get = |a: &AtomicU32| f32::from_bits(a.load(Ordering::Relaxed));
            (get(&self.singing), get(&self.rapping))
        }
    }

    impl Drop for VoiceDetector {
        fn drop(&mut self) {
            unsafe { self.analyzer.completeAnalysis() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rap_is_not_counted_as_singing() {
        let (s, r) = voices(|l| if l == "rapping" { 0.9 } else if l == "speech" { 0.8 } else { 0.0 });
        assert!(r > 0.85 && s < 0.1, "singing {s}, rapping {r}");
        let (s, r) = voices(|l| if l == "singing" { 0.7 } else if l == "speech" { 0.4 } else { 0.0 });
        assert!(s > 0.8 && r == 0.0, "singing {s}, rapping {r}");
    }

    #[test]
    fn any_voice_combines_independent_labels() {
        assert_eq!(any_of([0.0, 0.0]), 0.0);
        assert!((any_of([0.5, 0.5]) - 0.75).abs() < 1e-6);
        assert!((any_of([1.0, 0.2]) - 1.0).abs() < 1e-6);
    }
}
