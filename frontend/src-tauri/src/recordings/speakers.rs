//! "Me / Others" labels for transcript lines, from the separate tracks: whichever of
//! mic.mp4 (you) and system.mp4 (everyone else) is clearly louder during a line is who was
//! speaking. No second transcription is needed. Lines where neither side clearly dominates
//! (both talking, silence) are left unlabelled rather than guessed. With headphones the
//! tracks are clean; through speakers others also leak into the mic, which this tolerates
//! because the system track still carries them louder.

use crate::state::AppState;
use serde::Serialize;
use sqlx::SqlitePool;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SAMPLE_RATE: usize = 8000;
const WINDOW_SECONDS: f64 = 0.25;
/// One side must be this much louder to be called the speaker.
const MARGIN_DB: f64 = 6.0;
/// Below this, a window counts as silence.
const SILENCE_DB: f64 = -55.0;

/// Mean energy of each 0.25 s window of a recording, decoded with the bundled ffmpeg.
fn window_energies(ffmpeg: &Path, file: &Path) -> Result<Vec<f32>, String> {
    let mut child = Command::new(ffmpeg)
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(file)
        .args(["-ac", "1", "-ar", &SAMPLE_RATE.to_string(), "-f", "f32le", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not run ffmpeg: {}", e))?;
    let mut stdout = child.stdout.take().ok_or("No ffmpeg output")?;
    let window = (SAMPLE_RATE as f64 * WINDOW_SECONDS) as usize;
    let mut energies = Vec::new();
    let (mut sum, mut n) = (0f64, 0usize);
    let mut buf = vec![0u8; 64 * 1024];
    let mut carry: Vec<u8> = Vec::new();
    loop {
        let read = stdout.read(&mut buf).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        carry.extend_from_slice(&buf[..read]);
        let usable = carry.len() - carry.len() % 4;
        for c in carry[..usable].chunks_exact(4) {
            let s = f32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f64;
            sum += s * s;
            n += 1;
            if n == window {
                energies.push((sum / n as f64) as f32);
                sum = 0.0;
                n = 0;
            }
        }
        carry.drain(..usable);
    }
    if n > 0 {
        energies.push((sum / n as f64) as f32);
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        let mut err = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut err);
        }
        return Err(format!("ffmpeg could not read {}: {}", file.display(), err.trim()));
    }
    Ok(energies)
}

fn db(energy: f64) -> f64 {
    10.0 * (energy + 1e-12).log10()
}

fn mean_db(energies: &[f32], start: f64, end: f64) -> f64 {
    let a = ((start / WINDOW_SECONDS).floor() as usize).min(energies.len());
    let b = ((end / WINDOW_SECONDS).ceil() as usize).clamp(a, energies.len());
    if a == b {
        return SILENCE_DB - 1.0;
    }
    let mean = energies[a..b].iter().map(|&e| e as f64).sum::<f64>() / (b - a) as f64;
    db(mean)
}

/// "Me", "Others" or "" (unclear) for a line spanning start..end seconds.
pub fn label(mic: &[f32], system: &[f32], start: f64, end: f64) -> &'static str {
    let end = if end > start { end } else { start + 2.0 };
    let (m, s) = (mean_db(mic, start, end), mean_db(system, start, end));
    if m < SILENCE_DB && s < SILENCE_DB {
        ""
    } else if m - s > MARGIN_DB {
        "Me"
    } else if s - m > MARGIN_DB {
        "Others"
    } else {
        ""
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerResult {
    pub me: usize,
    pub others: usize,
    pub unclear: usize,
}

pub async fn label_meeting(pool: &SqlitePool, meeting_id: &str) -> Result<SpeakerResult, String> {
    let folder: Option<String> = sqlx::query_scalar::<_, Option<String>>("SELECT folder_path FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .flatten();
    let folder = folder.map(PathBuf::from).filter(|f| f.is_dir()).ok_or("This meeting has no recording folder")?;
    let (mic, system) = (folder.join("mic.mp4"), folder.join("system.mp4"));
    if !mic.is_file() || !system.is_file() {
        return Err("This recording has no separate mic and system tracks (it was made before they were added, or imported)".into());
    }
    let ffmpeg = crate::audio::ffmpeg::find_ffmpeg_path().ok_or("ffmpeg not found")?;
    let (mic_e, sys_e) = tokio::task::spawn_blocking(move || -> Result<(Vec<f32>, Vec<f32>), String> {
        Ok((window_energies(&ffmpeg, &mic)?, window_energies(&ffmpeg, &system)?))
    })
    .await
    .map_err(|e| e.to_string())??;

    let rows: Vec<(String, Option<f64>, Option<f64>)> =
        sqlx::query_as("SELECT id, audio_start_time, audio_end_time FROM transcripts WHERE meeting_id = ?")
            .bind(meeting_id)
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;
    let mut result = SpeakerResult { me: 0, others: 0, unclear: 0 };
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    for (id, start, end) in rows {
        let Some(start) = start else { continue };
        let l = label(&mic_e, &sys_e, start, end.unwrap_or(start + 2.0));
        match l {
            "Me" => result.me += 1,
            "Others" => result.others += 1,
            _ => result.unclear += 1,
        }
        // "" marks "looked at, unclear", so the meeting isn't labelled again and again.
        sqlx::query("UPDATE transcripts SET speaker = ? WHERE id = ?")
            .bind(l)
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    log::info!("Speakers for {}: {} me, {} others, {} unclear", meeting_id, result.me, result.others, result.unclear);
    Ok(result)
}

#[tauri::command]
pub async fn api_label_speakers(state: tauri::State<'_, AppState>, meeting_id: String) -> Result<SpeakerResult, String> {
    label_meeting(state.db_manager.pool(), &meeting_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(levels: &[(f64, f32)]) -> Vec<f32> {
        // (seconds, energy) blocks -> 0.25 s windows
        levels.iter().flat_map(|&(secs, e)| std::iter::repeat_n(e, (secs / WINDOW_SECONDS) as usize)).collect()
    }

    #[test]
    fn louder_side_is_the_speaker() {
        // 0-10 s: I talk (mic loud, others' track quiet); 10-20 s: others talk; 20-30 s: both; 30-40 s: silence
        let mic = track(&[(10.0, 0.01), (10.0, 0.0002), (10.0, 0.01), (10.0, 1e-9)]);
        let sys = track(&[(10.0, 0.0001), (10.0, 0.02), (10.0, 0.01), (10.0, 1e-9)]);
        assert_eq!(label(&mic, &sys, 1.0, 9.0), "Me");
        assert_eq!(label(&mic, &sys, 11.0, 19.0), "Others");
        assert_eq!(label(&mic, &sys, 21.0, 29.0), "", "both talking: no guess");
        assert_eq!(label(&mic, &sys, 31.0, 39.0), "", "silence: no guess");
        assert_eq!(label(&mic, &sys, 2.0, 2.0), "Me", "zero-length lines use a short window");
        assert_eq!(label(&mic, &sys, 100.0, 105.0), "", "past the end of the audio");
    }

    #[test]
    fn decodes_a_real_file_with_ffmpeg() {
        let Some(ffmpeg) = crate::audio::ffmpeg::find_ffmpeg_path() else { return };
        let dir = std::env::temp_dir().join(format!("hearken-spk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tone.mp4");
        // 2 s of a 440 Hz tone then 2 s of silence
        let ok = Command::new(&ffmpeg)
            .args(["-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=2", "-f", "lavfi", "-i", "anullsrc=r=48000:cl=mono", "-t", "4",
                   "-filter_complex", "[0][1]concat=n=2:v=0:a=1", "-c:a", "aac"])
            .arg(&file)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return; // this ffmpeg build lacks lavfi; nothing to test
        }
        let e = window_energies(&ffmpeg, &file).unwrap();
        assert!(e.len() >= 15, "about 16 windows for 4 s, got {}", e.len());
        // ffmpeg's test tone has amplitude 1/8: about -21 dB of energy
        assert!(mean_db(&e, 0.2, 1.8) > -30.0, "tone is loud: {} dB", mean_db(&e, 0.2, 1.8));
        assert!(mean_db(&e, 2.5, 3.5) < SILENCE_DB, "silence is quiet");
        std::fs::remove_dir_all(dir).ok();
    }
}
