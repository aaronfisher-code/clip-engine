use crate::models::{AudioTrack, Clip, ProbeResult, Selection};
use crate::paths::AppPaths;
use anyhow::{bail, Context};
use chrono::{DateTime, Utc};
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

fn rate(value: Option<&str>) -> f64 {
    let mut parts = value
        .unwrap_or_default()
        .split('/')
        .map(|part| part.parse::<f64>().unwrap_or(0.0));
    let numerator = parts.next().unwrap_or(0.0);
    let denominator = parts.next().unwrap_or(1.0);
    if denominator == 0.0 {
        0.0
    } else {
        numerator / denominator
    }
}

pub async fn probe(
    paths: &AppPaths,
    source: &Path,
    display_name: Option<String>,
) -> anyhow::Result<Clip> {
    let output = Command::new(&paths.ffprobe)
        .args([
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-of",
            "json",
        ])
        .arg(source)
        .output()
        .await
        .context("Could not start FFprobe")?;
    if !output.status.success() {
        bail!(
            "FFprobe failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let result: ProbeResult =
        serde_json::from_slice(&output.stdout).context("FFprobe returned invalid JSON")?;
    let video = result
        .streams
        .iter()
        .find(|stream| stream.codec_type == "video")
        .context("No video stream was found")?;
    let metadata = tokio::fs::metadata(source).await?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| DateTime::<Utc>::from(time).to_rfc3339().into());
    let created = result
        .format
        .tags
        .as_ref()
        .and_then(|tags| tags.creation_time.clone())
        .unwrap_or_else(|| modified.unwrap_or_else(|| Utc::now().to_rfc3339()));
    let modified_millis = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_millis())
        .unwrap_or(0);
    let fingerprint = format!(
        "{}:{}:{}",
        source.to_string_lossy(),
        metadata.len(),
        modified_millis
    );
    let audio_tracks = result
        .streams
        .iter()
        .filter(|stream| stream.codec_type == "audio")
        .enumerate()
        .map(|(ordinal, stream)| AudioTrack {
            stream_index: stream.index,
            ordinal: ordinal as i64,
            codec: stream
                .codec_name
                .clone()
                .unwrap_or_else(|| "unknown".into()),
            channels: stream.channels.unwrap_or(0),
            channel_layout: stream.channel_layout.clone(),
            title: stream.tags.as_ref().and_then(|tags| tags.title.clone()),
            language: stream.tags.as_ref().and_then(|tags| tags.language.clone()),
        })
        .collect();
    Ok(Clip {
        id: uuid::Uuid::new_v4().to_string(),
        name: display_name
            .or_else(|| {
                source
                    .file_name()
                    .map(|value| value.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| "Untitled clip".into()),
        source_path: source.to_string_lossy().to_string(),
        fingerprint,
        created_at: created,
        imported_at: Utc::now().to_rfc3339(),
        size: metadata.len() as i64,
        duration: result
            .format
            .duration
            .as_deref()
            .unwrap_or("0")
            .parse()
            .unwrap_or(0.0),
        width: video.width.unwrap_or(0),
        height: video.height.unwrap_or(0),
        fps: rate(video.avg_frame_rate.as_deref()).max(rate(video.r_frame_rate.as_deref())),
        video_codec: video.codec_name.clone().unwrap_or_else(|| "unknown".into()),
        audio_tracks,
        preview_status: "pending".into(),
        preview_path: None,
        preview_error: None,
    })
}

async fn run(paths: &AppPaths, args: &[String]) -> anyhow::Result<()> {
    let output = Command::new(&paths.ffmpeg)
        .args(args)
        .output()
        .await
        .context("Could not start FFmpeg")?;
    if !output.status.success() {
        bail!(
            "FFmpeg failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

pub async fn make_preview(
    paths: &AppPaths,
    source: &Path,
    output: &Path,
    thumbnail: &Path,
) -> anyhow::Result<()> {
    let temporary_output = output.with_extension("working.mp4");
    let temporary_thumbnail = thumbnail.with_extension("working.jpg");
    let args = vec![
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-i",
        &source.to_string_lossy(),
        "-map",
        "0:v:0",
        "-map",
        "0:a:0?",
        "-vf",
        "scale=960:-2:force_original_aspect_ratio=decrease:flags=fast_bilinear,fps=30",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "30",
        "-tune",
        "fastdecode",
        "-g",
        "15",
        "-keyint_min",
        "15",
        "-sc_threshold",
        "0",
        "-bf",
        "0",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-b:a",
        "96k",
        "-movflags",
        "+faststart",
        "-f",
        "mp4",
        &temporary_output.to_string_lossy(),
    ]
    .into_iter()
    .map(String::from)
    .collect::<Vec<_>>();
    if let Err(error) = run(paths, &args).await {
        let _ = tokio::fs::remove_file(&temporary_output).await;
        return Err(error);
    }
    let thumbnail_args = vec![
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-ss",
        "0.250",
        "-i",
        &source.to_string_lossy(),
        "-frames:v",
        "1",
        "-vf",
        "scale=320:-2:force_original_aspect_ratio=decrease:flags=fast_bilinear",
        "-q:v",
        "4",
        &temporary_thumbnail.to_string_lossy(),
    ]
    .into_iter()
    .map(String::from)
    .collect::<Vec<_>>();
    if let Err(error) = run(paths, &thumbnail_args).await {
        let _ = tokio::fs::remove_file(&temporary_output).await;
        let _ = tokio::fs::remove_file(&temporary_thumbnail).await;
        return Err(error);
    }
    let _ = tokio::fs::remove_file(output).await;
    let _ = tokio::fs::remove_file(thumbnail).await;
    tokio::fs::rename(&temporary_output, output).await?;
    tokio::fs::rename(&temporary_thumbnail, thumbnail).await?;
    Ok(())
}

pub fn safe_base_name(name: &str) -> String {
    let stem = Path::new(name)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let mut value = String::new();
    let mut dash = false;
    for character in stem.chars() {
        if character.is_ascii_alphanumeric() {
            value.push(character);
            dash = false;
        } else if !dash && !value.is_empty() {
            value.push('-');
            dash = true;
        }
        if value.len() >= 60 {
            break;
        }
    }
    value
        .trim_matches('-')
        .to_string()
        .chars()
        .take(60)
        .collect::<String>()
        .pipe(|value| {
            if value.is_empty() {
                "clip".into()
            } else {
                value
            }
        })
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

pub fn export_args(
    source: &Path,
    output: &Path,
    selection: &Selection,
    source_fps: f64,
    encoder: &str,
    quality: i64,
) -> Vec<String> {
    let duration = selection.end - selection.start;
    let output_fps = source_fps.clamp(1.0, 120.0).round() as i64;
    let mut args = vec![
        "-y".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "warning".into(),
        "-ss".into(),
        format!("{:.6}", selection.start),
        "-i".into(),
        source.to_string_lossy().to_string(),
        "-t".into(),
        format!("{duration:.6}"),
    ];
    if selection.audio_stream_indexes.len() > 1 {
        let inputs = selection
            .audio_stream_indexes
            .iter()
            .enumerate()
            .map(|(position, index)| {
                format!("[0:{index}]aresample=async=1:first_pts=0[a{position}]")
            })
            .collect::<Vec<_>>()
            .join(";");
        let pads = selection
            .audio_stream_indexes
            .iter()
            .enumerate()
            .map(|(position, _)| format!("[a{position}]"))
            .collect::<String>();
        args.extend([
            "-filter_complex".into(),
            format!(
                "{inputs};{pads}amix=inputs={}:duration=longest:normalize=1[aout]",
                selection.audio_stream_indexes.len()
            ),
        ]);
    }
    args.extend(["-map".into(), "0:v:0".into(), "-vf".into(), format!("scale=1920:1080:force_original_aspect_ratio=decrease:flags=lanczos,pad=1920:1080:(ow-iw)/2:(oh-ih)/2,fps={output_fps}")]);
    if encoder.ends_with("_nvenc") {
        args.extend(
            [
                "-c:v",
                encoder,
                "-preset",
                "p5",
                "-tune",
                "hq",
                "-rc",
                "vbr",
                "-cq",
                &quality.to_string(),
                "-b:v",
                "20M",
                "-maxrate",
                "30M",
                "-bufsize",
                "60M",
                "-g",
                &(output_fps * 2).to_string(),
                "-profile:v",
                "high",
            ]
            .into_iter()
            .map(String::from),
        );
    } else if encoder.ends_with("_qsv") {
        args.extend([
            "-c:v".into(),
            encoder.into(),
            "-preset".into(),
            "medium".into(),
            "-global_quality".into(),
            quality.to_string(),
        ]);
    } else if encoder.ends_with("_amf") {
        args.extend([
            "-c:v".into(),
            encoder.into(),
            "-quality".into(),
            "balanced".into(),
            "-rc".into(),
            "cqp".into(),
            "-qp_i".into(),
            quality.to_string(),
            "-qp_p".into(),
            quality.to_string(),
        ]);
    } else {
        args.extend([
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "medium".into(),
            "-crf".into(),
            quality.to_string(),
            "-maxrate".into(),
            "30M".into(),
            "-bufsize".into(),
            "60M".into(),
            "-profile:v".into(),
            "high".into(),
        ]);
    }
    args.extend(["-pix_fmt".into(), "yuv420p".into()]);
    match selection.audio_stream_indexes.as_slice() {
        [] => args.push("-an".into()),
        [index] => args.extend([
            "-map".into(),
            format!("0:{index}"),
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "192k".into(),
        ]),
        _ => args.extend([
            "-map".into(),
            "[aout]".into(),
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "192k".into(),
        ]),
    }
    args.extend([
        "-movflags".into(),
        "+faststart".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-nostats".into(),
        output.to_string_lossy().to_string(),
    ]);
    args
}

#[allow(clippy::too_many_arguments)]
pub async fn export_clip<F>(
    paths: &AppPaths,
    source: &Path,
    output: &Path,
    selection: &Selection,
    source_fps: f64,
    encoder: &str,
    quality: i64,
    mut progress: F,
) -> anyhow::Result<()>
where
    F: FnMut(f64) + Send,
{
    let mut child = Command::new(&paths.ffmpeg)
        .args(export_args(
            source, output, selection, source_fps, encoder, quality,
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Could not start FFmpeg")?;
    let stdout = child
        .stdout
        .take()
        .context("FFmpeg progress pipe was unavailable")?;
    let mut stderr = child
        .stderr
        .take()
        .context("FFmpeg error pipe was unavailable")?;
    let stderr_task = tokio::spawn(async move {
        let mut output = Vec::new();
        stderr.read_to_end(&mut output).await.map(|_| output)
    });
    let mut lines = BufReader::new(stdout).lines();
    let duration_us = (selection.end - selection.start) * 1_000_000.0;
    while let Some(line) = lines.next_line().await? {
        if let Some(value) = line.strip_prefix("out_time_us=") {
            if let Ok(value) = value.parse::<f64>() {
                progress((value / duration_us).clamp(0.0, 1.0));
            }
        }
    }
    let status = child.wait().await?;
    let stderr = stderr_task.await??;
    if !status.success() {
        bail!(
            "FFmpeg export failed: {}",
            String::from_utf8_lossy(&stderr).trim()
        );
    }
    progress(1.0);
    Ok(())
}

pub async fn make_thumbnail(
    paths: &AppPaths,
    video: &Path,
    output: &Path,
    duration: f64,
) -> anyhow::Result<()> {
    let seek = (duration * 0.25).min((duration - 0.1).max(0.0));
    let args = vec!["-y".into(), "-hide_banner".into(), "-loglevel".into(), "error".into(), "-ss".into(), format!("{seek:.3}"),
        "-i".into(), video.to_string_lossy().to_string(), "-frames:v".into(), "1".into(), "-vf".into(),
        "scale=1280:720:force_original_aspect_ratio=decrease:flags=lanczos,pad=1280:720:(ow-iw)/2:(oh-ih)/2".into(),
        "-q:v".into(), "2".into(), output.to_string_lossy().to_string()];
    run(paths, &args).await
}

pub async fn detect_encoder(paths: &AppPaths) -> String {
    for encoder in ["h264_nvenc", "h264_qsv", "h264_amf"] {
        let output = Command::new(&paths.ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=size=64x64:rate=30",
                "-frames:v",
                "1",
                "-c:v",
                encoder,
                "-f",
                "null",
                "-",
            ])
            .output()
            .await;
        if output.is_ok_and(|value| value.status.success()) {
            return encoder.into();
        }
    }
    "libx264".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_names_are_safe() {
        assert_eq!(safe_base_name("../../Round Win!!.mkv"), "round-win");
    }
    #[test]
    fn output_fps_never_upsamples_past_source() {
        let args = export_args(
            Path::new("in.mkv"),
            Path::new("out.mp4"),
            &Selection {
                start: 0.0,
                end: 1.0,
                audio_stream_indexes: vec![],
            },
            60.0,
            "libx264",
            20,
        );
        assert!(args.iter().any(|value| value.ends_with("fps=60")));
    }
}
