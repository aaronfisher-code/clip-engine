use crate::models::{AudioTrack, Clip, ExportProfile, ProbeResult, Selection};
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
    thumbnail: &Path,
    _duration: f64,
) -> anyhow::Result<()> {
    let temporary_thumbnail = thumbnail.with_extension("working.jpg");
    let thumbnail_args = vec![
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-i",
        &source.to_string_lossy(),
        "-frames:v",
        "1",
        "-vf",
        "scale=1920:-2:force_original_aspect_ratio=decrease:flags=lanczos,format=yuv420p",
        "-q:v",
        "2",
        &temporary_thumbnail.to_string_lossy(),
    ]
    .into_iter()
    .map(String::from)
    .collect::<Vec<_>>();
    if let Err(error) = run(paths, &thumbnail_args).await {
        let _ = tokio::fs::remove_file(&temporary_thumbnail).await;
        return Err(error);
    }
    let _ = tokio::fs::remove_file(thumbnail).await;
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

pub const MAX_PUBLISH_BYTES: u64 = 200 * 1024 * 1024;
const AUDIO_BITRATE: u64 = 192_000;
const REF_VIDEO_BITRATE: f64 = 20_000_000.0;
const REF_PIXEL_RATE: f64 = 1920.0 * 1080.0 * 120.0;
/// MP4 muxing, AAC frame padding, and encoder VBV overshoot above the average bitrate.
const SIZE_MARGIN: f64 = 1.12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishOption {
    pub width: i64,
    pub height: i64,
    pub fps: i64,
    pub video_bitrate: u64,
    pub estimated_bytes: u64,
}

impl PublishOption {
    pub fn profile(&self) -> ExportProfile {
        ExportProfile {
            width: self.width,
            height: self.height,
            fps: self.fps,
            video_bitrate: self.video_bitrate,
        }
    }

    pub fn quality_label(&self) -> String {
        format!("{}p{}", self.height, self.fps)
    }

    pub fn matches(&self, profile: &ExportProfile) -> bool {
        self.width == profile.width && self.height == profile.height && self.fps == profile.fps
    }

    pub fn heavier_than_1080p120(&self) -> bool {
        self.width.max(1) as f64 * self.height.max(1) as f64 * self.fps.max(1) as f64
            > REF_PIXEL_RATE
    }
}

pub fn format_file_size(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 10.0 {
        format!("{mb:.0} MB")
    } else if mb >= 0.1 {
        format!("{mb:.1} MB")
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    }
}

fn even(value: i64) -> i64 {
    value - value.rem_euclid(2)
}

fn output_size(source_width: i64, source_height: i64, target_height: i64) -> (i64, i64) {
    let height = even(target_height.max(2));
    let width = even(
        ((source_width as f64) * (height as f64) / (source_height.max(1) as f64)).round() as i64,
    )
    .max(2);
    (width, height)
}

fn height_steps(source_height: i64) -> Vec<i64> {
    let source = even(source_height.max(2));
    let mut heights = vec![source];
    for candidate in [2160, 1440, 1080, 720] {
        if candidate < source {
            heights.push(candidate);
        }
    }
    if source >= 720 {
        heights.retain(|height| *height >= 720);
    }
    heights.sort_unstable_by(|left, right| right.cmp(left));
    heights.dedup();
    heights
}

fn fps_steps(source_fps: f64) -> Vec<i64> {
    let source = source_fps.round().clamp(1.0, 240.0) as i64;
    let mut rates = vec![source];
    for candidate in [120, 60, 30] {
        if candidate < source {
            rates.push(candidate);
        }
    }
    rates.sort_unstable_by(|left, right| right.cmp(left));
    rates.dedup();
    rates
}

pub fn video_bitrate_for(width: i64, height: i64, fps: i64) -> u64 {
    let pixel_rate = width.max(1) as f64 * height.max(1) as f64 * fps.max(1) as f64;
    let bits = REF_VIDEO_BITRATE * (pixel_rate / REF_PIXEL_RATE);
    bits.round().clamp(800_000.0, 80_000_000.0) as u64
}

pub fn estimated_publish_bytes(
    video_bitrate: u64,
    duration: f64,
    has_audio: bool,
) -> u64 {
    let audio = if has_audio { AUDIO_BITRATE } else { 0 };
    let payload = (video_bitrate + audio) as f64 * duration.max(0.05) / 8.0;
    (payload * SIZE_MARGIN).ceil() as u64
}

pub fn publish_options(
    source_width: i64,
    source_height: i64,
    source_fps: f64,
    duration: f64,
    has_audio: bool,
) -> Vec<PublishOption> {
    let mut options = Vec::new();
    for height in height_steps(source_height) {
        let (width, height) = output_size(source_width, source_height, height);
        for fps in fps_steps(source_fps) {
            let video_bitrate = video_bitrate_for(width, height, fps);
            let estimated_bytes = estimated_publish_bytes(video_bitrate, duration, has_audio);
            if estimated_bytes > MAX_PUBLISH_BYTES {
                continue;
            }
            options.push(PublishOption {
                width,
                height,
                fps,
                video_bitrate,
                estimated_bytes,
            });
        }
    }
    options
}

pub fn resolve_export_profile(
    source_width: i64,
    source_height: i64,
    source_fps: f64,
    selection: &Selection,
) -> anyhow::Result<ExportProfile> {
    let duration = selection.end - selection.start;
    let has_audio = !selection.audio_stream_indexes.is_empty();
    let options = publish_options(
        source_width,
        source_height,
        source_fps,
        duration,
        has_audio,
    );
    if options.is_empty() {
        bail!("This selection is too long to publish under 200 MB, even at 720p30. Shorten the trim.");
    }
    if let Some(profile) = &selection.export {
        if let Some(option) = options.iter().find(|option| option.matches(profile)) {
            return Ok(option.profile());
        }
        bail!("That publish quality would exceed 200 MB for this selection.");
    }
    Ok(options[0].profile())
}

fn bitrate_arg(bits_per_second: u64) -> String {
    format!("{}k", (bits_per_second / 1000).max(1))
}

pub fn export_args(
    source: &Path,
    output: &Path,
    selection: &Selection,
    profile: &ExportProfile,
    encoder: &str,
    _quality: i64,
) -> Vec<String> {
    let duration = selection.end - selection.start;
    let output_fps = profile.fps.clamp(1, 240);
    let video_bitrate = if profile.video_bitrate == 0 {
        video_bitrate_for(profile.width, profile.height, output_fps)
    } else {
        profile.video_bitrate
    };
    let bitrate = bitrate_arg(video_bitrate);
    let maxrate = bitrate.clone();
    let bufsize = bitrate_arg(video_bitrate);
    let scale = format!(
        "scale={}:{}:force_original_aspect_ratio=decrease:flags=lanczos,pad={}:{}:(ow-iw)/2:(oh-ih)/2,fps={output_fps}",
        profile.width, profile.height, profile.width, profile.height
    );
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
    args.extend(["-map".into(), "0:v:0".into(), "-vf".into(), scale]);
    let gop = (output_fps * 2).to_string();
    if encoder.ends_with("_nvenc") {
        args.extend([
            "-c:v".into(),
            encoder.into(),
            "-preset".into(),
            "p5".into(),
            "-tune".into(),
            "hq".into(),
            "-rc".into(),
            "cbr".into(),
            "-b:v".into(),
            bitrate,
            "-maxrate".into(),
            maxrate,
            "-bufsize".into(),
            bufsize,
            "-g".into(),
            gop,
            "-profile:v".into(),
            "high".into(),
        ]);
    } else if encoder.ends_with("_qsv") {
        args.extend([
            "-c:v".into(),
            encoder.into(),
            "-preset".into(),
            "medium".into(),
            "-b:v".into(),
            bitrate,
            "-maxrate".into(),
            maxrate,
            "-bufsize".into(),
            bufsize,
        ]);
    } else if encoder.ends_with("_amf") {
        args.extend([
            "-c:v".into(),
            encoder.into(),
            "-quality".into(),
            "balanced".into(),
            "-rc".into(),
            "cbr".into(),
            "-b:v".into(),
            bitrate,
            "-maxrate".into(),
            maxrate,
            "-bufsize".into(),
            bufsize,
        ]);
    } else {
        args.extend([
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "medium".into(),
            "-b:v".into(),
            bitrate,
            "-maxrate".into(),
            maxrate.clone(),
            "-minrate".into(),
            maxrate,
            "-bufsize".into(),
            bufsize,
            "-x264-params".into(),
            "nal-hrd=cbr".into(),
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
    profile: &ExportProfile,
    encoder: &str,
    quality: i64,
    mut progress: F,
) -> anyhow::Result<()>
where
    F: FnMut(f64) + Send,
{
    let mut child = Command::new(&paths.ffmpeg)
        .args(export_args(
            source, output, selection, profile, encoder, quality,
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
    let seek = (duration * 0.5).min((duration - 0.1).max(0.0));
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
                "quiet",
                "-nostats",
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
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
        if output.is_ok_and(|status| status.success()) {
            return encoder.into();
        }
    }
    "libx264".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silent_selection(end: f64, export: ExportProfile) -> Selection {
        Selection {
            start: 0.0,
            end,
            audio_stream_indexes: vec![],
            export: Some(export),
        }
    }

    #[test]
    fn output_names_are_safe() {
        assert_eq!(safe_base_name("../../Round Win!!.mkv"), "round-win");
    }

    #[test]
    fn output_fps_never_upsamples_past_source() {
        let profile = ExportProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            video_bitrate: video_bitrate_for(1920, 1080, 60),
        };
        let args = export_args(
            Path::new("in.mkv"),
            Path::new("out.mp4"),
            &silent_selection(1.0, profile.clone()),
            &profile,
            "libx264",
            20,
        );
        assert!(args.iter().any(|value| value.ends_with("fps=60")));
        assert!(args.iter().any(|value| value.starts_with("scale=1920:1080:")));
        let bitrate = args.iter().position(|value| value == "-b:v").unwrap();
        let maxrate = args.iter().position(|value| value == "-maxrate").unwrap();
        assert_eq!(args[bitrate + 1], args[maxrate + 1]);
    }

    #[test]
    fn nvenc_uses_constrained_cbr() {
        let profile = ExportProfile {
            width: 2560,
            height: 1440,
            fps: 120,
            video_bitrate: video_bitrate_for(2560, 1440, 120),
        };
        let args = export_args(
            Path::new("in.mkv"),
            Path::new("out.mp4"),
            &silent_selection(1.0, profile.clone()),
            &profile,
            "h264_nvenc",
            20,
        );
        assert!(args.windows(2).any(|pair| pair == ["-rc", "cbr"]));
        assert!(!args.iter().any(|value| value == "-cq"));
    }

    #[test]
    fn size_estimate_includes_mux_and_overshoot_margin() {
        let bitrate = video_bitrate_for(2560, 1440, 120);
        let duration = 44.0;
        let raw = ((bitrate + 192_000) as f64 * duration / 8.0).ceil() as u64;
        let estimated = estimated_publish_bytes(bitrate, duration, true);
        assert!(estimated > raw);
        assert!(estimated > MAX_PUBLISH_BYTES);
        let options = publish_options(2560, 1440, 120.0, duration, true);
        assert!(!options.iter().any(|option| option.quality_label() == "1440p120"));
    }

    #[test]
    fn publish_ladder_steps_down_from_source_to_720p30() {
        let options = publish_options(2560, 1440, 120.0, 12.0, true);
        let labels = options
            .iter()
            .map(PublishOption::quality_label)
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            vec![
                "1440p120", "1440p60", "1440p30", "1080p120", "1080p60", "1080p30", "720p120",
                "720p60", "720p30"
            ]
        );
    }

    #[test]
    fn publish_ladder_does_not_upsample() {
        let options = publish_options(1920, 1080, 60.0, 8.0, true);
        let labels = options
            .iter()
            .map(PublishOption::quality_label)
            .collect::<Vec<_>>();
        assert_eq!(labels, vec!["1080p60", "1080p30", "720p60", "720p30"]);
        assert!(options.iter().all(|option| option.estimated_bytes <= MAX_PUBLISH_BYTES));
    }

    #[test]
    fn publish_ladder_hides_options_over_200mb() {
        let options = publish_options(1920, 1080, 120.0, 90.0, true);
        assert!(!options.is_empty());
        assert!(options.iter().all(|option| option.estimated_bytes <= MAX_PUBLISH_BYTES));
        assert!(!options.iter().any(|option| option.quality_label() == "1080p120"));
        assert!(options.iter().any(|option| option.quality_label() == "720p30"));
    }

    #[test]
    fn very_long_clips_can_have_no_publish_options() {
        let options = publish_options(1920, 1080, 120.0, 20.0 * 60.0, true);
        assert!(options.is_empty());
    }

    #[test]
    fn options_heavier_than_1080p120_are_flagged() {
        let options = publish_options(3840, 2160, 120.0, 6.0, true);
        let flag = |label: &str| {
            options
                .iter()
                .find(|option| option.quality_label() == label)
                .map(PublishOption::heavier_than_1080p120)
        };
        assert_eq!(flag("1080p120"), Some(false));
        assert_eq!(flag("2160p30"), Some(false));
        assert_eq!(flag("1440p60"), Some(false));
        assert_eq!(flag("1440p120"), Some(true));
        assert_eq!(flag("2160p60"), Some(true));
        assert_eq!(flag("2160p120"), Some(true));
    }
}
