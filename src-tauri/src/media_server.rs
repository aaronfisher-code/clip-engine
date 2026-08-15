use crate::database::Database;
use crate::models::Clip;
use anyhow::Context;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const MAX_REQUEST_BYTES: usize = 16 * 1024;

#[derive(Clone)]
pub struct MediaServer {
    pub base_url: String,
}

impl MediaServer {
    pub fn start(root: PathBuf, database: Database, ffmpeg: PathBuf) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .context("Could not start the local media server")?;
        let address = listener.local_addr()?;
        let token = uuid::Uuid::new_v4().to_string();
        let base_url = format!("http://127.0.0.1:{}/{token}", address.port());

        std::thread::Builder::new()
            .name("clip-media-server".into())
            .spawn(move || {
                for connection in listener.incoming() {
                    let Ok(connection) = connection else { continue };
                    let root = root.clone();
                    let database = database.clone();
                    let ffmpeg = ffmpeg.clone();
                    let token = token.clone();
                    let _ = std::thread::Builder::new()
                        .name("clip-media-request".into())
                        .spawn(move || {
                            let _ = serve(connection, &root, &database, &ffmpeg, &token);
                        });
                }
            })?;

        Ok(Self { base_url })
    }
}

fn serve(
    mut stream: TcpStream,
    root: &Path,
    database: &Database,
    ffmpeg: &Path,
    token: &str,
) -> anyhow::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;

    let request = read_request(&mut stream)?;
    let mut lines = request.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split_whitespace();
    let method = request_line.next().unwrap_or_default();
    let target = request_line.next().unwrap_or_default();
    if !matches!(method, "GET" | "HEAD") {
        return write_empty(&mut stream, "405 Method Not Allowed", None);
    }

    let expected_prefix = format!("/{token}/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let Some(route) = path.strip_prefix(&expected_prefix) else {
        return write_empty(&mut stream, "404 Not Found", None);
    };
    let range = lines.find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("range").then(|| value.trim())
    });

    if let Some(id) = route.strip_prefix("source/") {
        let Some(clip) = validated_clip(database, id)? else {
            return write_empty(&mut stream, "404 Not Found", None);
        };
        let path = PathBuf::from(&clip.source_path);
        return serve_file(
            &mut stream,
            method,
            &path,
            source_content_type(&path),
            range,
        );
    }

    if let Some(id) = route.strip_prefix("stream/") {
        let Some(clip) = validated_clip(database, id)? else {
            return write_empty(&mut stream, "404 Not Found", None);
        };
        return serve_compat_stream(&mut stream, method, ffmpeg, &clip, query);
    }

    if let Some(id) = route.strip_prefix("audio/") {
        let Some(clip) = validated_clip(database, id)? else {
            return write_empty(&mut stream, "404 Not Found", None);
        };
        return serve_audio_mix(&mut stream, method, ffmpeg, &clip, query);
    }

    if route.contains('/') || route.contains('\\') {
        return write_empty(&mut stream, "404 Not Found", None);
    }
    let Some((id, extension)) = route.rsplit_once('.') else {
        return write_empty(&mut stream, "404 Not Found", None);
    };
    if uuid::Uuid::parse_str(id).is_err() || !matches!(extension, "mp4" | "jpg") {
        return write_empty(&mut stream, "404 Not Found", None);
    }
    let file_path = root.join(route);
    let canonical_root = root.canonicalize()?;
    let Ok(canonical_file) = file_path.canonicalize() else {
        return write_empty(&mut stream, "404 Not Found", None);
    };
    if !canonical_file.starts_with(&canonical_root) {
        return write_empty(&mut stream, "403 Forbidden", None);
    }
    serve_file(
        &mut stream,
        method,
        &canonical_file,
        if extension == "mp4" {
            "video/mp4"
        } else {
            "image/jpeg"
        },
        range,
    )
}

fn read_request(stream: &mut TcpStream) -> anyhow::Result<String> {
    let mut request = Vec::with_capacity(2048);
    let mut chunk = [0_u8; 2048];
    while request.len() < MAX_REQUEST_BYTES {
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..count]);
        if request.windows(4).any(|value| value == b"\r\n\r\n") {
            break;
        }
    }
    if request.len() >= MAX_REQUEST_BYTES {
        anyhow::bail!("Request headers are too large");
    }
    Ok(String::from_utf8_lossy(&request).into_owned())
}

fn validated_clip(database: &Database, id: &str) -> anyhow::Result<Option<Clip>> {
    if uuid::Uuid::parse_str(id).is_err() {
        return Ok(None);
    }
    database.clip(id)
}

fn source_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        _ => "application/octet-stream",
    }
}

fn serve_file(
    stream: &mut TcpStream,
    method: &str,
    path: &Path,
    content_type: &str,
    range: Option<&str>,
) -> anyhow::Result<()> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return write_empty(stream, "404 Not Found", None),
    };
    let length = file.metadata()?.len();
    if length == 0 {
        return write_empty(stream, "404 Not Found", None);
    }
    let (status, start, end) = match range {
        Some(value) => match parse_range(value, length) {
            Some((start, end)) => ("206 Partial Content", start, end),
            None => {
                return write_empty(
                    stream,
                    "416 Range Not Satisfiable",
                    Some(&format!("Content-Range: bytes */{length}\r\n")),
                )
            }
        },
        None => ("200 OK", 0, length - 1),
    };
    let response_length = end - start + 1;
    let content_range = if status.starts_with("206") {
        format!("Content-Range: bytes {start}-{end}/{length}\r\n")
    } else {
        String::new()
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {response_length}\r\nAccept-Ranges: bytes\r\n{content_range}Cache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n"
    )?;
    if method == "GET" {
        file.seek(SeekFrom::Start(start))?;
        std::io::copy(&mut file.take(response_length), stream)?;
    }
    Ok(())
}

fn serve_compat_stream(
    stream: &mut TcpStream,
    method: &str,
    ffmpeg: &Path,
    clip: &Clip,
    query: &str,
) -> anyhow::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n"
    )?;
    stream.flush()?;
    if method == "HEAD" {
        return Ok(());
    }

    let start = query_value(query, "start")
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.0)
        .clamp(0.0, clip.duration.max(0.0));
    let audio = selected_audio_streams(clip, query);
    let mut args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-fflags".into(),
        "+nobuffer+flush_packets+discardcorrupt".into(),
        "-avioflags".into(),
        "direct".into(),
        "-flush_packets".into(),
        "1".into(),
        "-ss".into(),
        format!("{start:.6}"),
        "-readrate".into(),
        "1.25".into(),
        "-readrate_initial_burst".into(),
        "5".into(),
        "-readrate_catchup".into(),
        "2".into(),
        "-i".into(),
        clip.source_path.clone(),
    ];

    let mut filters = vec!["[0:v:0]format=yuv420p[video]".to_string()];
    if audio.len() > 1 {
        for (position, index) in audio.iter().enumerate() {
            filters.push(format!("[0:{index}]aresample=44100[a{position}]"));
        }
        let inputs = audio
            .iter()
            .enumerate()
            .map(|(position, _)| format!("[a{position}]"))
            .collect::<String>();
        filters.push(format!(
            "{inputs}amix=inputs={}:duration=longest:normalize=1[audio]",
            audio.len()
        ));
    } else if let Some(index) = audio.first() {
        filters.push(format!("[0:{index}]anull[audio]"));
    }
    args.extend(["-filter_complex".into(), filters.join(";")]);
    args.extend([
        "-map".into(),
        "[video]".into(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "ultrafast".into(),
        "-tune".into(),
        "zerolatency".into(),
        "-crf".into(),
        "18".into(),
        "-g".into(),
        "1".into(),
    ]);
    if audio.is_empty() {
        args.push("-an".into());
    } else {
        args.extend([
            "-map".into(),
            "[audio]".into(),
            "-ac".into(),
            "2".into(),
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "128k".into(),
        ]);
    }
    args.extend([
        "-fps_mode".into(),
        "passthrough".into(),
        "-f".into(),
        "mp4".into(),
        "-movflags".into(),
        "+frag_keyframe+empty_moov+default_base_moof".into(),
        "pipe:1".into(),
    ]);

    let mut child = Command::new(ffmpeg)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Could not start FFmpeg-assisted playback")?;
    if let Some(mut output) = child.stdout.take() {
        let _ = std::io::copy(&mut output, stream);
    }
    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}

fn serve_audio_mix(
    stream: &mut TcpStream,
    method: &str,
    ffmpeg: &Path,
    clip: &Clip,
    query: &str,
) -> anyhow::Result<()> {
    let audio = selected_audio_streams(clip, query);
    if audio.is_empty() {
        return write_empty(stream, "204 No Content", None);
    }

    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n"
    )?;
    stream.flush()?;
    if method == "HEAD" {
        return Ok(());
    }

    let start = query_value(query, "start")
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.0)
        .clamp(0.0, clip.duration.max(0.0));
    let mut args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-fflags".into(),
        "+nobuffer+flush_packets+discardcorrupt".into(),
        "-ss".into(),
        format!("{start:.6}"),
        "-readrate".into(),
        "1.25".into(),
        "-readrate_initial_burst".into(),
        "3".into(),
        "-readrate_catchup".into(),
        "2".into(),
        "-i".into(),
        clip.source_path.clone(),
    ];

    let mut filters = Vec::new();
    for (position, index) in audio.iter().enumerate() {
        filters.push(format!(
            "[0:{index}]aresample=48000:async=1:first_pts=0[a{position}]"
        ));
    }
    let inputs = audio
        .iter()
        .enumerate()
        .map(|(position, _)| format!("[a{position}]"))
        .collect::<String>();
    if audio.len() > 1 {
        filters.push(format!(
            "{inputs}amix=inputs={}:duration=longest:normalize=1[audio]",
            audio.len()
        ));
    } else {
        filters.push(format!("{inputs}anull[audio]"));
    }

    args.extend(["-filter_complex".into(), filters.join(";")]);
    args.extend([
        "-map".into(),
        "[audio]".into(),
        "-vn".into(),
        "-ac".into(),
        "2".into(),
        "-ar".into(),
        "48000".into(),
        "-c:a".into(),
        "libmp3lame".into(),
        "-b:a".into(),
        "192k".into(),
        "-f".into(),
        "mp3".into(),
        "-write_xing".into(),
        "0".into(),
        "pipe:1".into(),
    ]);

    let mut child = Command::new(ffmpeg)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Could not start the preview audio mixer")?;
    if let Some(mut output) = child.stdout.take() {
        let _ = std::io::copy(&mut output, stream);
    }
    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}

fn selected_audio_streams(clip: &Clip, query: &str) -> Vec<i64> {
    let requested_audio = query_value(query, "audio")
        .map(|value| {
            value
                .split(',')
                .filter_map(|index| index.parse::<i64>().ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let valid_audio = clip
        .audio_tracks
        .iter()
        .map(|track| track.stream_index)
        .collect::<std::collections::HashSet<_>>();
    requested_audio
        .into_iter()
        .filter(|index| valid_audio.contains(index))
        .collect()
}

fn query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == key).then_some(value)
    })
}

fn parse_range(value: &str, length: u64) -> Option<(u64, u64)> {
    let value = value.strip_prefix("bytes=")?;
    if value.contains(',') {
        return None;
    }
    let (start, end) = value.split_once('-')?;
    if start.is_empty() {
        let suffix = end.parse::<u64>().ok()?.min(length);
        return (suffix > 0).then_some((length - suffix, length - 1));
    }
    let start = start.parse::<u64>().ok()?;
    if start >= length {
        return None;
    }
    let end = if end.is_empty() {
        length - 1
    } else {
        end.parse::<u64>().ok()?.min(length - 1)
    };
    (end >= start).then_some((start, end))
}

fn write_empty(stream: &mut TcpStream, status: &str, extra: Option<&str>) -> anyhow::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: 0\r\n{}Cache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
        extra.unwrap_or_default()
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_range, query_value};

    #[test]
    fn parses_open_and_suffix_ranges() {
        assert_eq!(parse_range("bytes=10-", 100), Some((10, 99)));
        assert_eq!(parse_range("bytes=10-19", 100), Some((10, 19)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=100-", 100), None);
    }

    #[test]
    fn parses_stream_query_values() {
        assert_eq!(query_value("start=1.5&audio=1,2", "start"), Some("1.5"));
        assert_eq!(query_value("start=1.5&audio=1,2", "audio"), Some("1,2"));
    }
}
