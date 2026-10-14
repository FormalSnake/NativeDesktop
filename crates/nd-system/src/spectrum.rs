//! The spectrum every player reports: [`BINS`] log-spaced bands from 50 Hz to 16 kHz, each
//! 0 at -60 dB and 1 at full scale. [`Analyzer`] computes it from samples (the macOS tap);
//! [`fold_db_bands`] folds GStreamer's linear `spectrum` bands into the same bins (Linux).

use std::f32::consts::PI;

pub const BINS: usize = 32;
pub const FFT_SIZE: usize = 1024;
pub const LOW_HZ: f64 = 50.;
pub const HIGH_HZ: f64 = 16_000.;
pub const FLOOR_DB: f32 = -60.;
/// Per-frame release of the peak envelope: fast attack, slow release steadies the bars without
/// smearing peaks.
pub const DECAY: f32 = 0.90;

fn band_edges(b: usize) -> (f64, f64) {
    let ratio = HIGH_HZ / LOW_HZ;
    let lo = LOW_HZ * ratio.powf(b as f64 / BINS as f64);
    let hi = LOW_HZ * ratio.powf((b + 1) as f64 / BINS as f64);
    (lo, hi)
}

fn normalize(db: f32) -> f32 {
    ((db - FLOOR_DB) / -FLOOR_DB).clamp(0., 1.)
}

/// A windowed 1024-point FFT over mono samples, folded into [`BINS`] bands with a peak
/// envelope. Allocates only in [`Analyzer::new`], so [`Analyzer::process`] can run on a
/// realtime audio thread.
pub struct Analyzer {
    window: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    ranges: Vec<(usize, usize)>,
    envelope: [f32; BINS],
}

impl Analyzer {
    pub fn new(sample_rate: f64) -> Self {
        let n = FFT_SIZE as f64;
        let max_bin = FFT_SIZE / 2 - 1;
        let ranges = (0..BINS)
            .map(|b| {
                let (lo_hz, hi_hz) = band_edges(b);
                // Bands above the signal's Nyquist collapse onto the top bin instead of inverting.
                let lo = ((lo_hz * n / sample_rate).floor() as usize).clamp(1, max_bin);
                let hi = ((hi_hz * n / sample_rate).ceil() as usize).clamp(lo, max_bin);
                (lo, hi)
            })
            .collect();
        let window = (0..FFT_SIZE).map(|i| 0.5 - 0.5 * (2. * PI * i as f32 / FFT_SIZE as f32).cos()).collect();
        Self { window, re: vec![0.; FFT_SIZE], im: vec![0.; FFT_SIZE], ranges, envelope: [0.; BINS] }
    }

    /// Analyzes the first [`FFT_SIZE`] samples of `samples` (zero padded when shorter) and
    /// returns the updated bins.
    pub fn process(&mut self, samples: impl IntoIterator<Item = f32>) -> [f32; BINS] {
        self.re.fill(0.);
        self.im.fill(0.);
        for (i, s) in samples.into_iter().take(FFT_SIZE).enumerate() {
            self.re[i] = s * self.window[i];
        }
        fft(&mut self.re, &mut self.im);
        // The Hann window halves a tone's amplitude and a real FFT splits it over two bins, so
        // 4/N brings a full-scale sine to about unity.
        let scale = 4. / FFT_SIZE as f32;
        for (b, &(lo, hi)) in self.ranges.iter().enumerate() {
            let peak =
                (lo..=hi).map(|k| (self.re[k] * self.re[k] + self.im[k] * self.im[k]).sqrt()).fold(0f32, f32::max);
            let db = 20. * (peak * scale + 1e-9).log10();
            let norm = normalize(db);
            let e = &mut self.envelope[b];
            *e = if norm > *e { norm } else { *e * DECAY };
        }
        self.envelope
    }
}

/// In-place radix-2 FFT; `re.len()` must be a power of two.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -2. * PI / len as f32;
        let (w_re, w_im) = (angle.cos(), angle.sin());
        for start in (0..n).step_by(len) {
            let (mut cur_re, mut cur_im) = (1f32, 0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let t_re = re[b] * cur_re - im[b] * cur_im;
                let t_im = re[b] * cur_im + im[b] * cur_re;
                re[b] = re[a] - t_re;
                im[b] = im[a] - t_im;
                re[a] += t_re;
                im[a] += t_im;
                let next = cur_re * w_re - cur_im * w_im;
                cur_im = cur_re * w_im + cur_im * w_re;
                cur_re = next;
            }
        }
        len <<= 1;
    }
}

/// Folds GStreamer `spectrum` magnitudes (dB per linear band from 0 Hz to `nyquist_hz`) into
/// [`BINS`]: each bin takes the loudest band it covers.
pub fn fold_db_bands(bands_db: &[f32], nyquist_hz: f64) -> [f32; BINS] {
    let mut out = [0.; BINS];
    let n = bands_db.len();
    if n == 0 {
        return out;
    }
    for (b, slot) in out.iter_mut().enumerate() {
        let (f0, f1) = band_edges(b);
        let lo = ((f0 / nyquist_hz * n as f64).floor() as usize).min(n - 1);
        let hi = ((f1 / nyquist_hz * n as f64).ceil() as usize).max(lo + 1).min(n);
        let peak = bands_db[lo..hi].iter().copied().fold(f32::NEG_INFINITY, f32::max);
        *slot = if peak.is_finite() { normalize(peak) } else { 0. };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, rate: f32, amplitude: f32) -> Vec<f32> {
        (0..FFT_SIZE).map(|i| amplitude * (2. * PI * hz * i as f32 / rate).sin()).collect()
    }

    fn loudest(bins: &[f32; BINS]) -> usize {
        bins.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i).unwrap()
    }

    #[test]
    fn a_tone_lights_its_band() {
        for hz in [100., 1_000., 8_000.] {
            let mut a = Analyzer::new(48_000.);
            let bins = a.process(tone(hz, 48_000., 1.));
            let expected = (0..BINS).find(|&b| band_edges(b).0 <= hz as f64 && (hz as f64) < band_edges(b).1).unwrap();
            // Below a few hundred Hz neighbouring bands share FFT bins, so several light together.
            assert!(bins[expected] > 0.9, "{hz} Hz reads {} in its band", bins[expected]);
            let far = (0..BINS).filter(|b| b.abs_diff(expected) > 6).map(|b| bins[b]).fold(0f32, f32::max);
            assert!(far < 0.5, "{hz} Hz leaks {far} into distant bands");
        }
    }

    #[test]
    fn silence_reads_zero_and_peaks_decay() {
        let mut a = Analyzer::new(44_100.);
        assert!(a.process(std::iter::repeat_n(0., FFT_SIZE)).iter().all(|&b| b == 0.));
        let lit = a.process(tone(1_000., 44_100., 1.));
        let after = a.process(std::iter::repeat_n(0., FFT_SIZE));
        let b = loudest(&lit);
        assert!((after[b] - lit[b] * DECAY).abs() < 1e-6, "release is the decay, not a drop");
    }

    #[test]
    fn quiet_tone_scales_by_db() {
        let mut a = Analyzer::new(48_000.);
        // -30 dB is halfway between the floor and full scale.
        let bins = a.process(tone(1_000., 48_000., 10f32.powf(-30. / 20.)));
        let peak = bins[loudest(&bins)];
        assert!((peak - 0.5).abs() < 0.08, "-30 dB reads {peak}");
    }

    #[test]
    fn folds_gstreamer_bands() {
        let mut bands = vec![-80.; 128];
        // 128 bands to 22.05 kHz: band 6 covers about 1.03 to 1.21 kHz.
        bands[6] = 0.;
        let bins = fold_db_bands(&bands, 22_050.);
        let at = (0..BINS).find(|&b| band_edges(b).0 <= 1_100. && 1_100. < band_edges(b).1).unwrap();
        assert_eq!(bins[at], 1.);
        assert_eq!((bins[0], bins[BINS - 1]), (0., 0.));
        assert_eq!(fold_db_bands(&[], 22_050.), [0.; BINS]);
    }
}
