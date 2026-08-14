import { useCallback, useEffect, useRef, useState, type DragEvent } from "react";
import { api } from "./api";
import type { AppConfig, Clip, Job } from "./types";

function formatDuration(seconds: number) {
  if (!Number.isFinite(seconds)) return "0:00.000";
  const minutes = Math.floor(seconds / 60);
  return `${minutes}:${(seconds % 60).toFixed(3).padStart(6, "0")}`;
}

function formatBytes(bytes: number) {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${(bytes / 1024 ** index).toFixed(index > 1 ? 1 : 0)} ${units[index]}`;
}

function trackName(track: Clip["audioTracks"][number]) {
  return track.title || track.language || `Audio ${track.ordinal + 1}`;
}

export function App() {
  const [config, setConfig] = useState<AppConfig>();
  const [clips, setClips] = useState<Clip[]>([]);
  const [selectedId, setSelectedId] = useState<string>();
  const [jobs, setJobs] = useState<Job[]>([]);
  const [busy, setBusy] = useState(false);
  const [dragActive, setDragActive] = useState(false);
  const [notice, setNotice] = useState<string>();
  const [error, setError] = useState<string>();
  const fileInput = useRef<HTMLInputElement>(null);
  const dragDepth = useRef(0);

  const refresh = useCallback(async () => {
    const [nextClips, nextJobs] = await Promise.all([api.clips(), api.jobs()]);
    setClips(nextClips);
    setJobs(nextJobs);
    setSelectedId((current) => current || nextClips[0]?.id);
  }, []);

  useEffect(() => {
    void Promise.all([api.config(), api.clips(), api.jobs()])
      .then(([nextConfig, nextClips, nextJobs]) => {
        setConfig(nextConfig);
        setClips(nextClips);
        setJobs(nextJobs);
        setSelectedId(nextClips[0]?.id);
      })
      .catch((reason) => setError(reason.message));
  }, []);

  const hasActiveWork = clips.some((clip) => ["pending", "processing"].includes(clip.previewStatus))
    || jobs.some((job) => ["queued", "transcoding", "uploading"].includes(job.status));
  useEffect(() => {
    if (!hasActiveWork) return;
    const timer = window.setInterval(() => void refresh(), 1_000);
    return () => window.clearInterval(timer);
  }, [hasActiveWork, refresh]);

  const selected = clips.find((clip) => clip.id === selectedId);
  const selectedJobs = jobs.filter((job) => job.clipId === selectedId);

  async function run(action: () => Promise<void>) {
    setBusy(true);
    setError(undefined);
    setNotice(undefined);
    try {
      await action();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  }

  function importRecordings(files: FileList | File[]) {
    if (!files.length) return;
    void run(async () => {
      const result = await api.import(files);
      await refresh();
      setSelectedId(result.clips[0]?.id);
      setNotice(`${result.clips.length} recording${result.clips.length === 1 ? "" : "s"} imported.`);
    });
  }

  function isFileDrag(event: DragEvent) {
    return Array.from(event.dataTransfer.types).includes("Files");
  }

  function handleDragEnter(event: DragEvent<HTMLDivElement>) {
    if (!isFileDrag(event)) return;
    event.preventDefault();
    dragDepth.current += 1;
    setDragActive(true);
  }

  function handleDragOver(event: DragEvent<HTMLDivElement>) {
    if (!isFileDrag(event)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
  }

  function handleDragLeave(event: DragEvent<HTMLDivElement>) {
    if (!isFileDrag(event)) return;
    event.preventDefault();
    dragDepth.current = Math.max(0, dragDepth.current - 1);
    if (dragDepth.current === 0) setDragActive(false);
  }

  function handleDrop(event: DragEvent<HTMLDivElement>) {
    event.preventDefault();
    dragDepth.current = 0;
    setDragActive(false);
    if (busy) return;
    importRecordings(event.dataTransfer.files);
  }

  return (
    <div
      className={`app-shell ${dragActive ? "dragging" : ""}`}
      onDragEnter={handleDragEnter}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark"><i /></span>
          <div>
            <strong>Clip Engine</strong>
            <span>Local 120 fps workflow</span>
          </div>
        </div>
        <div className={`cloud-status ${config?.r2Configured ? "online" : ""}`}>
          <span />
          {config?.r2Configured ? config.publicBaseUrl : "R2 setup needed"}
        </div>
      </header>

      {dragActive && (
        <div className="drop-overlay" aria-hidden="true">
          <div>
            <span>↓</span>
            <strong>Drop recordings to import</strong>
            <small>MP4, MKV, MOV, WebM, AVI or M4V</small>
          </div>
        </div>
      )}

      <main className="workspace">
        <aside className="library">
          <div className="section-title">
            <div><span>Library</span><small>{clips.length} clips</small></div>
            <button className="icon-button" title="Scan recording folder" disabled={busy} onClick={() => void run(async () => {
              const result = await api.scan();
              setClips(result.clips);
              setSelectedId((current) => current || result.clips[0]?.id);
              setNotice(`Recording folder scanned — ${result.count} clip${result.count === 1 ? "" : "s"} available.`);
            })}>↻</button>
          </div>

          <button className="import-button" disabled={busy} onClick={() => fileInput.current?.click()}>
            <span>＋</span> Import recordings
          </button>
          <input ref={fileInput} hidden type="file" multiple accept="video/*,.mkv" onChange={(event) => {
            const files = event.target.files;
            if (!files?.length) return;
            importRecordings(files);
            event.target.value = "";
          }} />

          <div className="source-path" title={config?.sourceDirectory}>
            <span>Watching</span>{config?.sourceDirectory || "Loading…"}
          </div>

          <div className="clip-list">
            {clips.map((clip) => (
              <button key={clip.id} className={`clip-row ${clip.id === selectedId ? "active" : ""}`} onClick={() => setSelectedId(clip.id)}>
                <div className="thumb">
                  {clip.previewStatus === "ready" ? <video src={`/api/clips/${clip.id}/media#t=0.1`} preload="metadata" muted /> : <span>{clip.previewStatus === "failed" ? "!" : "···"}</span>}
                  <em>{formatDuration(clip.duration)}</em>
                </div>
                <div className="clip-copy">
                  <strong>{clip.name}</strong>
                  <span>{clip.width}×{clip.height} · {Math.round(clip.fps)} fps</span>
                  <small>{new Date(clip.createdAt).toLocaleString()}</small>
                </div>
              </button>
            ))}
            {!clips.length && (
              <div className="empty-library">
                <span>⌁</span>
                <strong>No recordings yet</strong>
                <p>Import a file or place OBS clips in the watched folder.</p>
              </div>
            )}
          </div>
        </aside>

        <section className="editor-panel">
          {(error || notice) && <div className={`toast ${error ? "error" : ""}`}><span>{error ? "!" : "✓"}</span>{error || notice}<button onClick={() => { setError(undefined); setNotice(undefined); }}>×</button></div>}
          {selected ? (
            <Editor key={selected.id} clip={selected} jobs={selectedJobs} config={config} onPublish={(start, end, tracks) => run(async () => {
              const job = await api.publish(selected.id, start, end, tracks);
              setJobs((current) => [job, ...current]);
              setNotice("Export queued. You can keep editing while it runs.");
            })} />
          ) : (
            <div className="welcome">
              <span className="welcome-mark">▶</span>
              <h1>Your replay buffer, refined.</h1>
              <p>Bring in a recording to trim it, choose the audio you want, and publish a clean 1080p120 share link.</p>
              <button onClick={() => fileInput.current?.click()}>Import your first recording</button>
            </div>
          )}
        </section>
      </main>
    </div>
  );
}

function Editor({ clip, jobs, config, onPublish }: {
  clip: Clip;
  jobs: Job[];
  config?: AppConfig;
  onPublish: (start: number, end: number, tracks: number[]) => void;
}) {
  const frameStep = 1 / Math.max(1, clip.fps || 120);
  const [start, setStart] = useState(0);
  const [end, setEnd] = useState(clip.duration);
  const [tracks, setTracks] = useState<number[]>(clip.audioTracks.map((track) => track.streamIndex));
  const video = useRef<HTMLVideoElement>(null);
  const activeJob = jobs.find((job) => ["queued", "transcoding", "uploading"].includes(job.status));
  const completedJob = jobs.find((job) => job.status === "complete" && job.url);

  function seek(time: number) {
    if (video.current) video.current.currentTime = time;
  }

  function changeStart(value: number) {
    const next = Math.max(0, Math.min(value, end - frameStep));
    setStart(next);
    seek(next);
  }

  function changeEnd(value: number) {
    const next = Math.min(clip.duration, Math.max(value, start + frameStep));
    setEnd(next);
    seek(next);
  }

  return (
    <div className="editor">
      <div className="editor-heading">
        <div>
          <span className="eyebrow">Editing recording</span>
          <h1>{clip.name}</h1>
          <p>{clip.width}×{clip.height} <b>·</b> {clip.fps.toFixed(2)} fps <b>·</b> {clip.videoCodec.toUpperCase()} <b>·</b> {formatBytes(clip.size)}</p>
        </div>
        <span className="output-pill">Output&nbsp; {config?.export.width || 1920}×{config?.export.height || 1080} / {config?.export.fps || 120} fps</span>
      </div>

      <div className="preview-stage">
        {clip.previewStatus === "ready" ? (
          <video ref={video} src={`/api/clips/${clip.id}/media`} controls playsInline onTimeUpdate={(event) => {
            if (event.currentTarget.currentTime >= end) event.currentTarget.pause();
          }} />
        ) : clip.previewStatus === "failed" ? (
          <div className="preview-message failed"><strong>Preview failed</strong><span>{clip.previewError}</span></div>
        ) : (
          <div className="preview-message"><i /><strong>Preparing browser preview</strong><span>The original recording stays untouched.</span></div>
        )}
      </div>

      <section className="trim-card">
        <div className="card-heading">
          <div><span className="step">01</span><div><strong>Trim</strong><small>Select the moment worth keeping</small></div></div>
          <span className="selection-length">{formatDuration(end - start)} selected</span>
        </div>
        <div className="timeline">
          <div className="timeline-track" />
          <div className="selection" style={{ left: `${(start / clip.duration) * 100}%`, right: `${100 - (end / clip.duration) * 100}%` }} />
          <input aria-label="Trim start" type="range" min="0" max={clip.duration} step={frameStep} value={start} onChange={(event) => changeStart(Number(event.target.value))} />
          <input aria-label="Trim end" type="range" min="0" max={clip.duration} step={frameStep} value={end} onChange={(event) => changeEnd(Number(event.target.value))} />
        </div>
        <div className="time-inputs">
          <label><span>In point</span><input type="number" min="0" max={end - frameStep} step={frameStep} value={start.toFixed(3)} onChange={(event) => changeStart(Number(event.target.value))} /><small>seconds</small></label>
          <button onClick={() => { setStart(0); setEnd(clip.duration); }}>Reset</button>
          <label><span>Out point</span><input type="number" min={start + frameStep} max={clip.duration} step={frameStep} value={end.toFixed(3)} onChange={(event) => changeEnd(Number(event.target.value))} /><small>seconds</small></label>
        </div>
      </section>

      <section className="audio-card">
        <div className="card-heading">
          <div><span className="step">02</span><div><strong>Audio mix</strong><small>Selected tracks are mixed for reliable playback</small></div></div>
          <span className="selection-length">{tracks.length} of {clip.audioTracks.length} tracks</span>
        </div>
        <div className="track-grid">
          {clip.audioTracks.map((track) => {
            const selected = tracks.includes(track.streamIndex);
            return <button key={track.streamIndex} className={`track ${selected ? "selected" : ""}`} onClick={() => setTracks((current) => selected ? current.filter((index) => index !== track.streamIndex) : [...current, track.streamIndex])}>
              <span className="check">{selected ? "✓" : ""}</span>
              <span className="track-icon">≋</span>
              <span><strong>{trackName(track)}</strong><small>{track.codec.toUpperCase()} · {track.channelLayout || `${track.channels} channels`}</small></span>
            </button>;
          })}
          {!clip.audioTracks.length && <p className="no-audio">This recording has no audio tracks. It will be exported silently.</p>}
        </div>
      </section>

      <section className="publish-card">
        <div className="publish-copy">
          <span className="step">03</span>
          <div><strong>Transcode & publish</strong><small>{config?.export.codec || "libx264"} · CRF {config?.export.crf || 20} · fast-start MP4</small></div>
        </div>
        {activeJob ? (
          <div className="job-progress">
            <div><span>{activeJob.status === "uploading" ? "Uploading to R2" : "Transcoding locally"}</span><strong>{Math.round(activeJob.progress * 100)}%</strong></div>
            <progress value={activeJob.progress} max="1" />
          </div>
        ) : (
          <button className="publish-button" disabled={!config?.r2Configured} title={!config?.r2Configured ? "Add R2 credentials to .env first" : ""} onClick={() => onPublish(start, end, tracks)}>
            <span>↑</span> Publish clip
          </button>
        )}
      </section>

      {jobs.find((job) => job.status === "failed")?.error && <div className="job-error"><strong>Publish failed</strong>{jobs.find((job) => job.status === "failed")?.error}</div>}
      {completedJob?.url && (
        <div className="share-result">
          <span>✓</span>
          <div><strong>Your clip is live</strong><a href={completedJob.url} target="_blank" rel="noreferrer">{completedJob.url}</a></div>
          <button onClick={() => void navigator.clipboard.writeText(completedJob.url!)}>Copy link</button>
        </div>
      )}
    </div>
  );
}
