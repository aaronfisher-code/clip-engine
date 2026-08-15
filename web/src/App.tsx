import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent, type PointerEvent as ReactPointerEvent } from "react";
import { api } from "./api";
import type { AccessRequest, AdminUser, AppConfig, Clip, CloudClip, CloudUser, Job } from "./types";

type AuthMode = "request" | "login";

type CreatedAccessLink = {
  username: string;
  token: string;
  url: string;
  expiresAt: string;
  purpose: "password_reset";
};

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

function expiryLabel(expiresAt?: string) {
  if (!expiresAt) return "Expiry pending";
  const milliseconds = new Date(expiresAt).getTime() - Date.now();
  if (milliseconds <= 0) return "Expired";
  const days = Math.ceil(milliseconds / 86_400_000);
  return `Expires in ${days} day${days === 1 ? "" : "s"}`;
}

function trackName(track: Clip["audioTracks"][number], configuredLabels: string[] = []) {
  const embeddedTitle = track.title?.trim();
  const titleIsGeneric = embeddedTitle
    ? /^(?:(?:audio\s*)?track|audio)\s*\d+$/i.test(embeddedTitle)
    : false;
  if (embeddedTitle && !titleIsGeneric) return embeddedTitle;

  const configuredLabel = configuredLabels[track.ordinal]?.trim();
  if (configuredLabel) return configuredLabel;

  const language = track.language?.trim();
  if (language && language.toLowerCase() !== "und") return language;
  return `Audio ${track.ordinal + 1}`;
}

export function App() {
  const [config, setConfig] = useState<AppConfig>();
  const [clips, setClips] = useState<Clip[]>([]);
  const [selectedId, setSelectedId] = useState<string>();
  const [jobs, setJobs] = useState<Job[]>([]);
  const [libraryArea, setLibraryArea] = useState<"inbox" | "published" | "team">("inbox");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string>();
  const [error, setError] = useState<string>();
  const [user, setUser] = useState<CloudUser>();
  const [cloudClips, setCloudClips] = useState<CloudClip[]>([]);
  const [adminUsers, setAdminUsers] = useState<AdminUser[]>([]);
  const [adminRequests, setAdminRequests] = useState<AccessRequest[]>([]);
  const [accessRequest, setAccessRequest] = useState<AccessRequest>();
  const [showAccess, setShowAccess] = useState(false);
  const [showAuth, setShowAuth] = useState<AuthMode>();
  const [accountOpen, setAccountOpen] = useState(false);
  const [libraryOpen, setLibraryOpen] = useState(() => window.localStorage.getItem("clip-engine-library") !== "closed");
  const [createdInvite, setCreatedInvite] = useState<CreatedAccessLink>();
  const [availableUpdate, setAvailableUpdate] = useState<string>();

  const refresh = useCallback(async () => {
    const [nextClips, nextJobs] = await Promise.all([api.clips(), api.jobs()]);
    setClips(nextClips);
    setJobs(nextJobs);
    setSelectedId((current) => nextClips.some((clip) => clip.id === current) ? current : nextClips[0]?.id);
  }, []);

  const refreshCloud = useCallback(async () => {
    setCloudClips(await api.cloudClips());
  }, []);

  useEffect(() => {
    void Promise.all([api.config(), api.clips(), api.jobs()])
      .then(async ([nextConfig, nextClips, nextJobs]) => {
        setConfig(nextConfig);
        setClips(nextClips);
        setJobs(nextJobs);
        const publishedIds = new Set(nextJobs.filter((job) => job.status === "complete" && job.url).map((job) => job.clipId));
        const firstClip = nextClips.find((clip) => !publishedIds.has(clip.id)) || nextClips[0];
        setSelectedId(firstClip?.id);
        setLibraryArea(firstClip && publishedIds.has(firstClip.id) ? "published" : "inbox");
        if (nextConfig.authenticated) {
          try {
            const [nextUser, nextCloudClips] = await Promise.all([api.me(), api.cloudClips()]);
            setUser(nextUser);
            setCloudClips(nextCloudClips);
          } catch {
            await api.logout().catch(() => undefined);
            const loggedOutConfig = await api.config();
            setConfig(loggedOutConfig);
            setShowAuth("login");
            setNotice("Your saved login expired. Sign in again to publish.");
          }
        } else if (nextConfig.pendingAccessRequest) {
          try { setAccessRequest(await api.accessRequestStatus()); }
          catch {
            await api.clearAccessRequest().catch(() => undefined);
            setConfig(await api.config());
            setShowAuth("request");
          }
        } else {
          setShowAuth("request");
        }
      })
      .catch((reason) => setError(reason.message));
    const updateTimer = window.setTimeout(() => {
      void api.checkForUpdate().then((update) => setAvailableUpdate(update?.version)).catch(() => undefined);
    }, 4_000);
    return () => window.clearTimeout(updateTimer);
  }, []);

  useEffect(() => {
    window.localStorage.setItem("clip-engine-library", libraryOpen ? "open" : "closed");
  }, [libraryOpen]);

  const hasActiveWork = clips.some((clip) => clip.previewStatus === "processing")
    || jobs.some((job) => ["queued", "transcoding", "uploading"].includes(job.status));
  useEffect(() => {
    if (!hasActiveWork) return;
    const timer = window.setInterval(() => void refresh(), 1_000);
    return () => window.clearInterval(timer);
  }, [hasActiveWork, refresh]);

  const selected = clips.find((clip) => clip.id === selectedId);
  const selectedJobs = jobs.filter((job) => job.clipId === selectedId);
  const publishedJobs = new Map<string, Job>();
  const publishedVersionCounts = new Map<string, number>();
  for (const job of jobs) {
    if (job.status === "complete" && job.url) {
      if (!publishedJobs.has(job.clipId)) publishedJobs.set(job.clipId, job);
      publishedVersionCounts.set(job.clipId, (publishedVersionCounts.get(job.clipId) || 0) + 1);
    }
  }
  const inboxClips = clips.filter((clip) => !publishedJobs.has(clip.id));
  const publishedClips = clips.filter((clip) => publishedJobs.has(clip.id));
  const visibleClips = libraryArea === "published" ? publishedClips : inboxClips;

  useEffect(() => {
    if (selectedId && publishedJobs.has(selectedId)) setLibraryArea("published");
  }, [jobs, selectedId]);

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

  function importRecordings() {
    void run(async () => {
      const result = await api.chooseRecordings();
      if (!result.clips.length) return;
      await refresh();
      setLibraryArea("inbox");
      setSelectedId(result.clips[0]?.id);
      setNotice(`${result.clips.length} recording${result.clips.length === 1 ? "" : "s"} imported.`);
    });
  }

  async function finishSignIn(session: { user: CloudUser }) {
    setUser(session.user);
    setConfig(await api.config());
    setAccessRequest(undefined);
    await refreshCloud();
    setShowAuth(undefined);
    setNotice(`Signed in as ${session.user.displayName}.`);
  }

  function manageAccess() {
    void run(async () => {
      const [users, requests] = await Promise.all([api.adminUsers(), api.adminAccessRequests()]);
      setAdminUsers(users);
      setAdminRequests(requests);
      setShowAccess(true);
    });
  }

  function chooseLibraryArea(area: "inbox" | "published" | "team") {
    setLibraryArea(area);
    if (area === "team") {
      setSelectedId(undefined);
      if (config?.authenticated) void run(refreshCloud);
      return;
    }
    const candidates = area === "published" ? publishedClips : inboxClips;
    setSelectedId((current) => candidates.some((clip) => clip.id === current) ? current : candidates[0]?.id);
  }

  function copyPublishedLink(clipId: string) {
    const url = publishedJobs.get(clipId)?.url;
    if (!url) return;
    void api.copyText(url)
      .then(() => setNotice("Published link copied."))
      .catch((reason) => setError(reason instanceof Error ? reason.message : String(reason)));
  }

  return (
    <div className="app-shell">
      <header className="topbar">
        <div className="topbar-left">
          <button className={`library-toggle ${libraryOpen ? "active" : ""}`} aria-pressed={libraryOpen} onClick={() => setLibraryOpen((open) => !open)}>
            <span>☰</span> Library <em>{clips.length}</em>
          </button>
          <div className="brand">
            <span className="brand-mark"><i /></span>
            <div>
              <strong>Dabs Clip Engine</strong>
              <span>Local 120 fps workflow</span>
            </div>
          </div>
        </div>
        <div className="account-actions">
          {availableUpdate && <button className="update-button" onClick={() => void run(async () => {
            setNotice(`Installing Clip Engine ${availableUpdate}…`);
            await api.installUpdate();
          })}>Update {availableUpdate}</button>}
          {user?.role === "owner" && <button onClick={manageAccess}>Manage access</button>}
          <button className={`cloud-status ${config?.authenticated ? "online" : ""}`} onClick={() => config?.authenticated ? setAccountOpen((open) => !open) : accessRequest ? undefined : setShowAuth("login")}>
            <span />{config?.authenticated ? user?.displayName || "Connected" : accessRequest?.status === "pending" ? "Approval pending" : "Sign in to publish"}
          </button>
          {accountOpen && user && <div className="account-menu">
            <strong>{user.displayName}</strong><span>@{user.username || "username not configured"}</span>
            <button onClick={() => void run(async () => {
              await api.logout(); setUser(undefined); setCloudClips([]); setAccountOpen(false); setConfig(await api.config());
              setShowAuth("login");
              setNotice("Signed out on this device.");
            })}>Sign out this device</button>
          </div>}
        </div>
      </header>

      <main className={`workspace ${libraryOpen ? "" : "library-closed"}`}>
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

          <button className="import-button" disabled={busy} onClick={importRecordings}>
            <span>＋</span> Import recordings
          </button>

          <div className="source-path" title={config?.sourceDirectory}>
            <span>Inbox</span>{config?.sourceDirectory || "Loading…"}
          </div>

          <div className="library-tabs" role="tablist" aria-label="Clip areas">
            <button role="tab" aria-selected={libraryArea === "inbox"} className={libraryArea === "inbox" ? "active" : ""} onClick={() => chooseLibraryArea("inbox")}>
              Inbox <span>{inboxClips.length}</span>
            </button>
            <button role="tab" aria-selected={libraryArea === "published"} className={libraryArea === "published" ? "active" : ""} onClick={() => chooseLibraryArea("published")}>
              Published <span>{publishedClips.length}</span>
            </button>
            <button role="tab" aria-selected={libraryArea === "team"} className={libraryArea === "team" ? "active" : ""} disabled={!config?.authenticated} onClick={() => chooseLibraryArea("team")}>
              Team <span>{cloudClips.length}</span>
            </button>
          </div>

          <div className="clip-list">
            {libraryArea !== "team" && visibleClips.map((clip) => (
              <div key={clip.id} className={`clip-row-wrap ${libraryArea === "published" ? "published" : ""}`}>
                <button className={`clip-row ${clip.id === selectedId ? "active" : ""}`} onClick={() => setSelectedId(clip.id)}>
                  <div className="thumb">
                    {clip.previewStatus === "ready" ? <img src={api.thumbnailUrl(clip, config)} loading="lazy" alt="" /> : <span>{clip.previewStatus === "failed" ? "!" : "···"}</span>}
                    <em>{formatDuration(clip.duration)}</em>
                  </div>
                  <div className="clip-copy">
                    <strong>{clip.name}</strong>
                    <span>{clip.width}×{clip.height} · {Math.round(clip.fps)} fps</span>
                    <small>{libraryArea === "published"
                      ? `${publishedVersionCounts.get(clip.id)} published version${publishedVersionCounts.get(clip.id) === 1 ? "" : "s"}`
                      : new Date(clip.createdAt).toLocaleString()}</small>
                  </div>
                </button>
                {publishedJobs.get(clip.id)?.url && <button className="quick-link" title="Copy published link" aria-label={`Copy published link for ${clip.name}`} onClick={() => copyPublishedLink(clip.id)}>⧉</button>}
              </div>
            ))}
            {libraryArea === "team" && cloudClips.map((clip) => (
              <button key={clip.id} className="cloud-clip-row" onClick={() => clip.url && void api.openExternal(clip.url)}>
                <div className="cloud-thumb">{clip.thumbnailUrl ? <img src={clip.thumbnailUrl} alt="" /> : <span>▶</span>}</div>
                <div className="clip-copy">
                  <strong>{clip.title}</strong>
                  <span>{clip.ownerName} · {formatDuration(clip.duration)}</span>
                  <small>{expiryLabel(clip.expiresAt)}</small>
                </div>
              </button>
            ))}
            {(libraryArea === "team" ? !cloudClips.length : !visibleClips.length) && (
              <div className="empty-library">
                <span>⌁</span>
                <strong>{libraryArea === "team" ? "No active team clips" : libraryArea === "published" ? "Nothing published yet" : "Inbox is clear"}</strong>
                <p>{libraryArea === "team" ? "Published clips from invited members appear here until they expire." : libraryArea === "published" ? "Completed exports move here with their share links." : "Import a file or save a new OBS replay."}</p>
              </div>
            )}
          </div>
        </aside>

        <section className="editor-panel">
          {(error || notice) && <div className={`toast ${error ? "error" : ""}`}><span>{error ? "!" : "✓"}</span>{error || notice}<button onClick={() => { setError(undefined); setNotice(undefined); }}>×</button></div>}
          {libraryArea === "team" ? (
            <CloudLibrary clips={cloudClips} user={user} busy={busy} onRefresh={() => void run(refreshCloud)} onExtend={(clip) => void run(async () => {
              const expiresAt = await api.extendCloudClip(clip.id);
              await refreshCloud();
              setNotice(`“${clip.title}” now expires ${new Date(expiresAt).toLocaleString()}.`);
            })} />
          ) : selected ? (
            <Editor key={selected.id} clip={selected} jobs={selectedJobs} config={config} busy={busy} onDelete={() => {
              const confirmed = window.confirm(`Remove "${selected.name}" from Clip Engine?\n\nThis deletes its disposable preview, local exports, and local history. The original recording and existing Cloudflare uploads stay untouched.`);
              if (!confirmed) return;
              void run(async () => {
                await api.remove(selected.id);
                setClips((current) => current.filter((clip) => clip.id !== selected.id));
                setJobs((current) => current.filter((job) => job.clipId !== selected.id));
                setSelectedId(visibleClips.find((clip) => clip.id !== selected.id)?.id);
                setNotice("Clip removed from the local library. The original recording and R2 uploads were kept.");
              });
            }} onDeleteVersion={(job) => {
              const confirmed = window.confirm(`Delete this published version of "${selected.name}"?\n\nThis permanently removes its share page, video, thumbnail, local export, and history. Other versions and the source recording will be kept.`);
              if (!confirmed) return;
              void run(async () => {
                await api.removeJob(job.id);
                setJobs((current) => current.filter((item) => item.id !== job.id));
                if (selectedJobs.filter((item) => item.status === "complete" && item.url).length === 1) {
                  setLibraryArea("inbox");
                }
                setNotice("Published version deleted from R2 and local history.");
              });
            }} onPublish={(start, end, tracks) => run(async () => {
              const job = await api.publish(selected.id, start, end, tracks);
              setJobs((current) => [job, ...current]);
              setNotice("Export queued. You can keep editing while it runs.");
            })} />
          ) : (
            <div className="welcome">
              <span className="welcome-mark">▶</span>
              <h1>Your replay buffer, refined.</h1>
              <p>Bring in a recording to trim it, choose the audio you want, and publish a clean 1080p120 share link.</p>
              <button onClick={importRecordings}>Import your first recording</button>
            </div>
          )}
        </section>
      </main>
      {showAuth && <AuthPanel initialMode={showAuth} busy={busy} onClose={() => setShowAuth(undefined)}
        onLogin={(username, password) => void run(async () => finishSignIn(await api.login(username, password, navigator.platform || "Desktop")))}
        onRequest={(username, displayName, password) => void run(async () => {
          const requested = await api.requestAccess(username, displayName, password);
          setAccessRequest(requested);
          setConfig(await api.config());
          setShowAuth(undefined);
          setNotice("Your request was sent to the owner for approval.");
        })}
        onValidateReset={(token, username) => api.validatePasswordReset(token, username)}
        onReset={(token, username, password) => void run(async () => finishSignIn(await api.redeemInvite(token, username, password, "", navigator.platform || "Desktop")))} />}
      {accessRequest && !config?.authenticated && !showAuth && <PendingAccessPanel request={accessRequest} busy={busy}
        onRefresh={() => void run(async () => setAccessRequest(await api.accessRequestStatus()))}
        onSignIn={() => setShowAuth("login")}
        onStartOver={() => void run(async () => {
          await api.clearAccessRequest(); setAccessRequest(undefined); setConfig(await api.config()); setShowAuth("request");
        })} />}
      {showAccess && <AccessPanel users={adminUsers} requests={adminRequests} currentUserId={user?.id} busy={busy} onClose={() => setShowAccess(false)} onChange={(member, status) => void run(async () => {
        await api.setUserStatus(member.id, status);
        setAdminUsers(await api.adminUsers());
        setNotice(`${member.displayName} is now ${status}.`);
      })} onReset={(member) => void run(async () => {
        const invite = await api.createPasswordReset(member.id);
        setCreatedInvite(invite);
        try { await api.copyText(invite.url); setNotice(`Password-reset link for @${member.username} copied.`); }
        catch { setNotice("Password-reset link created. Copy it from the open window."); }
      })} onReview={(request, decision) => void run(async () => {
        await api.reviewAccessRequest(request.id, decision);
        const [users, requests] = await Promise.all([api.adminUsers(), api.adminAccessRequests()]);
        setAdminUsers(users); setAdminRequests(requests);
        setNotice(`@${request.username} was ${decision}.`);
      })} />}
      {createdInvite && <InvitePanel invite={createdInvite} onClose={() => setCreatedInvite(undefined)} onCopy={() => void run(async () => {
        await api.copyText(createdInvite.url);
        setNotice("Private link copied.");
      })} />}
    </div>
  );
}

function AuthPanel({ initialMode, busy, onClose, onLogin, onRequest, onValidateReset, onReset }: {
  initialMode: AuthMode;
  busy: boolean;
  onClose: () => void;
  onLogin: (username: string, password: string) => void;
  onRequest: (username: string, displayName: string, password: string) => void;
  onValidateReset: (token: string, username: string) => Promise<void>;
  onReset: (token: string, username: string, password: string) => void;
}) {
  const [mode, setMode] = useState<AuthMode>(initialMode);
  const [forgotStep, setForgotStep] = useState<"closed" | "token" | "password">("closed");
  const [username, setUsername] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [password, setPassword] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [resetToken, setResetToken] = useState("");
  const [validating, setValidating] = useState(false);
  const [formError, setFormError] = useState<string>();

  function submit(event: FormEvent) {
    event.preventDefault();
    setFormError(undefined);
    if (mode === "request" && password !== confirmation) {
      setFormError("Passwords do not match.");
      return;
    }
    if (mode === "login") onLogin(username, password);
    else onRequest(username, displayName, password);
  }

  function chooseMode(next: AuthMode) {
    setMode(next);
    setForgotStep("closed");
    setFormError(undefined);
  }

  async function validateReset(event: FormEvent) {
    event.preventDefault();
    setFormError(undefined);
    setValidating(true);
    try {
      await onValidateReset(resetToken, username);
      setPassword("");
      setConfirmation("");
      setForgotStep("password");
    } catch (reason) {
      setFormError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setValidating(false);
    }
  }

  function finishReset(event: FormEvent) {
    event.preventDefault();
    setFormError(undefined);
    if (password !== confirmation) {
      setFormError("Passwords do not match.");
      return;
    }
    onReset(resetToken, username, password);
  }

  return <div className="modal-backdrop" role="presentation" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
    <section className="auth-panel" role="dialog" aria-modal="true" aria-label="Clip Engine account">
      <div className="access-heading"><div><span className="eyebrow">Private publishing access</span><h2>{forgotStep === "token" ? "Forgot your password?" : forgotStep === "password" ? "Choose a new password" : mode === "login" ? "Sign in" : "Create account"}</h2><p>{forgotStep === "token" ? "Ask the owner to generate a forgotten-password token, then enter it below." : forgotStep === "password" ? "Your token is valid. Enter the new password for this account." : mode === "login" ? "Use your username and password. The owner signs in as admin." : "Create an account request for the owner to review. No email required."}</p></div><button onClick={onClose}>×</button></div>
      {forgotStep === "closed" && <div className="auth-tabs simple" role="tablist"><button type="button" className={mode === "request" ? "active" : ""} onClick={() => chooseMode("request")}>Create account</button><button type="button" className={mode === "login" ? "active" : ""} onClick={() => chooseMode("login")}>Sign in</button></div>}
      {forgotStep === "closed" && <form className="auth-form" onSubmit={submit}>
        <label><span>Username</span><input required autoFocus minLength={3} maxLength={32} pattern="[A-Za-z0-9][A-Za-z0-9_-]{2,31}" value={username} onChange={(event) => setUsername(event.target.value)} autoCapitalize="none" autoComplete="username" placeholder={mode === "login" ? "Username" : "Choose a username"} /></label>
        {mode === "request" && <label><span>Display name</span><input required maxLength={100} value={displayName} onChange={(event) => setDisplayName(event.target.value)} autoComplete="nickname" placeholder="What friends will see" /></label>}
        <label><span>Password</span><input required type="password" minLength={12} maxLength={128} value={password} onChange={(event) => setPassword(event.target.value)} autoComplete={mode === "login" ? "current-password" : "new-password"} /></label>
        {mode === "request" && <label><span>Confirm password</span><input required type="password" minLength={12} maxLength={128} value={confirmation} onChange={(event) => setConfirmation(event.target.value)} autoComplete="new-password" /></label>}
        {formError && <p className="form-error">{formError}</p>}
        {mode === "login" && <button type="button" className="forgot-link" onClick={() => { setForgotStep("token"); setPassword(""); setFormError(undefined); }}>Forgot my password</button>}
        <button className="auth-submit" disabled={busy}>{busy ? "Please wait…" : mode === "login" ? "Sign in" : "Request access"}</button>
      </form>}
      {forgotStep === "token" && <form className="auth-form" onSubmit={(event) => void validateReset(event)}>
        <label><span>Username</span><input required autoFocus minLength={3} maxLength={32} value={username} onChange={(event) => setUsername(event.target.value)} autoCapitalize="none" /></label>
        <label><span>Forgotten-password token</span><textarea required value={resetToken} onChange={(event) => setResetToken(event.target.value)} placeholder="Paste the token or private reset link from the owner" /></label>
        {formError && <p className="form-error">{formError}</p>}
        <div className="auth-submit-row"><button type="button" onClick={() => { setForgotStep("closed"); setFormError(undefined); }}>Back</button><button className="auth-submit" disabled={validating}>{validating ? "Checking…" : "Continue"}</button></div>
      </form>}
      {forgotStep === "password" && <form className="auth-form" onSubmit={finishReset}>
        <label><span>New password</span><input required autoFocus type="password" minLength={12} maxLength={128} value={password} onChange={(event) => setPassword(event.target.value)} autoComplete="new-password" /></label>
        <label><span>Confirm new password</span><input required type="password" minLength={12} maxLength={128} value={confirmation} onChange={(event) => setConfirmation(event.target.value)} autoComplete="new-password" /></label>
        {formError && <p className="form-error">{formError}</p>}
        <button className="auth-submit" disabled={busy}>{busy ? "Resetting…" : "Reset password and sign in"}</button>
      </form>}
    </section>
  </div>;
}

function PendingAccessPanel({ request, busy, onRefresh, onSignIn, onStartOver }: { request: AccessRequest; busy: boolean; onRefresh: () => void; onSignIn: () => void; onStartOver: () => void }) {
  return <div className="modal-backdrop pending-backdrop"><section className="pending-panel" role="dialog" aria-modal="true" aria-label="Publishing access status">
    <span className={`pending-mark ${request.status}`}>{request.status === "pending" ? "…" : request.status === "approved" ? "✓" : "×"}</span>
    <span className="eyebrow">@{request.username}</span>
    <h2>{request.status === "pending" ? "Waiting for owner approval" : request.status === "approved" ? "Your access was approved" : "Your request was declined"}</h2>
    <p>{request.status === "pending" ? "Your account is in the owner's review queue. The owner will notify you once access is granted; return here and check your status." : request.status === "approved" ? "Sign in with the password you chose to activate this device. Your access remains active until the owner revokes it." : "You can start over with a different username or ask the owner why the request was declined."}</p>
    <div>{request.status === "pending" && <button disabled={busy} onClick={onRefresh}>{busy ? "Checking…" : "Check status"}</button>}{request.status === "approved" && <button onClick={onSignIn}>Sign in</button>}{request.status === "denied" && <button onClick={onStartOver}>Start over</button>}</div>
  </section></div>;
}

function CloudLibrary({ clips, user, busy, onRefresh, onExtend }: {
  clips: CloudClip[];
  user?: CloudUser;
  busy: boolean;
  onRefresh: () => void;
  onExtend: (clip: CloudClip) => void;
}) {
  return <div className="cloud-library">
    <div className="cloud-library-heading">
      <div><span className="eyebrow">Shared library</span><h1>Team clips</h1><p>Public links, private publishing access. Clips expire automatically after 30 days.</p></div>
      <button disabled={busy} onClick={onRefresh}>Refresh</button>
    </div>
    <div className="cloud-grid">
      {clips.map((clip) => <article key={clip.id}>
        <button className="cloud-card-media" onClick={() => clip.url && void api.openExternal(clip.url)}>
          {clip.thumbnailUrl ? <img src={clip.thumbnailUrl} alt="" /> : <span>▶</span>}
        </button>
        <div className="cloud-card-copy"><strong>{clip.title}</strong><span>{clip.ownerName} · {formatDuration(clip.duration)} · {formatBytes(clip.size)}</span><small>{expiryLabel(clip.expiresAt)}</small></div>
        <div className="cloud-card-actions">
          <button disabled={!clip.url} onClick={() => clip.url && void api.openExternal(clip.url)}>Open</button>
          <button disabled={busy} onClick={() => clip.url && void api.copyText(clip.url)}>Copy link</button>
          <button disabled={busy || (user?.role !== "owner" && clip.ownerId !== user?.id)} title={user?.role !== "owner" && clip.ownerId !== user?.id ? "Only the publisher or owner can extend this clip" : ""} onClick={() => onExtend(clip)}>Keep 30 days</button>
        </div>
      </article>)}
    </div>
    {!clips.length && <div className="welcome compact"><span className="welcome-mark">⌁</span><h1>No active clips</h1><p>Once someone publishes, the clip will show here for everyone with access.</p></div>}
  </div>;
}

function AccessPanel({ users, requests, currentUserId, busy, onClose, onChange, onReset, onReview }: {
  users: AdminUser[];
  requests: AccessRequest[];
  currentUserId?: string;
  busy: boolean;
  onClose: () => void;
  onChange: (user: AdminUser, status: "active" | "revoked") => void;
  onReset: (user: AdminUser) => void;
  onReview: (request: AccessRequest, decision: "approved" | "denied") => void;
}) {
  const pendingCount = requests.filter((request) => request.status === "pending").length;
  const [filter, setFilter] = useState<"pending" | "active" | "revoked" | "denied">(pendingCount ? "pending" : "active");
  const [query, setQuery] = useState("");
  const matches = (username: string, displayName: string) => `${username} ${displayName}`.toLowerCase().includes(query.trim().toLowerCase());
  const visibleRequests = requests.filter((request) => request.status === filter && matches(request.username, request.displayName));
  const visibleUsers = users.filter((member) => member.status === filter && matches(member.username, member.displayName));
  const empty = filter === "pending" || filter === "denied" ? !visibleRequests.length : !visibleUsers.length;
  return <div className="modal-backdrop" role="presentation" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
    <section className="access-panel" role="dialog" aria-modal="true" aria-label="Publishing access">
      <div className="access-heading"><div><span className="eyebrow">Owner controls</span><h2>Publishing access</h2><p>Review new accounts and revoke existing access from one place.</p></div><button onClick={onClose}>×</button></div>
      <div className="access-filters"><div><button className={filter === "pending" ? "active" : ""} onClick={() => setFilter("pending")}>Pending <span>{pendingCount}</span></button><button className={filter === "active" ? "active" : ""} onClick={() => setFilter("active")}>Active <span>{users.filter((user) => user.status === "active").length}</span></button><button className={filter === "revoked" ? "active" : ""} onClick={() => setFilter("revoked")}>Revoked <span>{users.filter((user) => user.status === "revoked").length}</span></button><button className={filter === "denied" ? "active" : ""} onClick={() => setFilter("denied")}>Declined</button></div><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Filter username…" /></div>
      <div className="access-list">
        {(filter === "pending" || filter === "denied") && visibleRequests.map((request) => <div className="access-row" key={request.id}>
          <div><strong>{request.displayName}</strong><span>@{request.username}</span><small>{request.status === "pending" ? `Requested ${new Date(request.createdAt).toLocaleString()}` : `Declined ${request.reviewedAt ? new Date(request.reviewedAt).toLocaleString() : ""}`}</small></div>
          {request.status === "pending" && <div className="access-actions"><button disabled={busy} className="deny" onClick={() => onReview(request, "denied")}>Decline</button><button disabled={busy} className="approve" onClick={() => onReview(request, "approved")}>Approve</button></div>}
        </div>)}
        {(filter === "active" || filter === "revoked") && visibleUsers.map((member) => <div className="access-row" key={member.id}>
          <div><strong>{member.displayName}</strong><span>@{member.username}</span><small>{member.role} · {member.deviceCount} device{member.deviceCount === 1 ? "" : "s"} · {member.activeClipCount} active clips / {formatBytes(member.activeBytes)} · {formatBytes(member.uploadedBytes)} uploaded total{member.lastSeenAt ? ` · seen ${new Date(member.lastSeenAt).toLocaleString()}` : ""}</small></div>
          <div className="access-actions"><button disabled={busy || member.id === currentUserId || member.status !== "active"} onClick={() => onReset(member)}>Reset password</button><button disabled={busy || member.id === currentUserId} className={member.status === "active" ? "revoke" : "restore"} onClick={() => onChange(member, member.status === "active" ? "revoked" : "active")}>{member.status === "active" ? "Revoke" : "Restore"}</button></div>
        </div>)}
        {empty && <div className="access-empty">No matching {filter} accounts.</div>}
      </div>
    </section>
  </div>;
}

function InvitePanel({ invite, onClose, onCopy }: {
  invite: CreatedAccessLink;
  onClose: () => void;
  onCopy: () => void;
}) {
  return <div className="modal-backdrop" role="presentation" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
    <section className="invite-panel" role="dialog" aria-modal="true" aria-label="Created password reset">
      <div className="access-heading"><div><span className="eyebrow">Password reset created</span><h2>@{invite.username}</h2><p>Single use · expires {new Date(invite.expiresAt).toLocaleString()}</p></div><button onClick={onClose}>×</button></div>
      <div className="invite-token"><label htmlFor="invite-token">Send this link privately</label><textarea id="invite-token" readOnly value={invite.url} onFocus={(event) => event.currentTarget.select()} /><p>This link lets that member replace their password and revokes their older device sessions.</p><div><button onClick={onCopy}>Copy link</button><button onClick={onClose}>Done</button></div></div>
    </section>
  </div>;
}

function Editor({ clip, jobs, config, busy, onDelete, onDeleteVersion, onPublish }: {
  clip: Clip;
  jobs: Job[];
  config?: AppConfig;
  busy: boolean;
  onDelete: () => void;
  onDeleteVersion: (job: Job) => void;
  onPublish: (start: number, end: number, tracks: number[]) => void;
}) {
  const frameStep = 1 / Math.max(1, clip.fps || 120);
  const [start, setStart] = useState(0);
  const [end, setEnd] = useState(clip.duration);
  const [tracks, setTracks] = useState<number[]>(clip.audioTracks.map((track) => track.streamIndex));
  const [currentTime, setCurrentTime] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [muted, setMuted] = useState(false);
  const [playbackMode, setPlaybackMode] = useState<"source" | "assisted" | "proxy">("source");
  const [assistedActive, setAssistedActive] = useState(false);
  const [streamStart, setStreamStart] = useState(0);
  const [streamVersion, setStreamVersion] = useState(0);
  const [audioMixActive, setAudioMixActive] = useState(false);
  const [audioMixStart, setAudioMixStart] = useState(0);
  const [audioMixVersion, setAudioMixVersion] = useState(0);
  const [mediaState, setMediaState] = useState<"loading" | "ready" | "error">("loading");
  const [mediaError, setMediaError] = useState<string>();
  const [buffering, setBuffering] = useState(false);
  const [showFrozenFrame, setShowFrozenFrame] = useState(false);
  const video = useRef<HTMLVideoElement>(null);
  const mixedAudio = useRef<HTMLAudioElement>(null);
  const frozenFrame = useRef<HTMLCanvasElement>(null);
  const seekInFlight = useRef(false);
  const queuedSeek = useRef<number | undefined>(undefined);
  const seekSafetyTimer = useRef<number | undefined>(undefined);
  const assistedSeekTimer = useRef<number | undefined>(undefined);
  const bufferingTimer = useRef<number | undefined>(undefined);
  const previewStopTimer = useRef<number | undefined>(undefined);
  const wantsToPlay = useRef(false);
  const playAfterSeek = useRef(false);
  const playWhenAudioMixReady = useRef(false);
  const allowAutomaticFallback = useRef(true);
  const timelineScrubbing = useRef(false);
  const timelineWasPlaying = useRef(false);
  const activeJob = jobs.find((job) => ["queued", "transcoding", "uploading"].includes(job.status));
  const completedJobs = jobs.filter((job) => job.status === "complete" && job.url);
  const usesExternalAudioMix = playbackMode !== "assisted"
    && Boolean(config?.mediaBaseUrl)
    && clip.audioTracks.length > 0;
  const assistedUrl = useMemo(() => config
    ? api.assistedStreamUrl(clip, config, streamStart, tracks, streamVersion)
    : undefined, [clip.id, config?.mediaBaseUrl, streamStart, streamVersion, tracks]);
  const playbackUrl = playbackMode === "source"
    ? api.sourceUrl(clip, config)
    : playbackMode === "proxy"
      ? api.previewUrl(clip, config)
      : assistedActive ? assistedUrl : undefined;
  const audioMixUrl = useMemo(() => config && audioMixActive
    ? api.audioMixUrl(clip, config, audioMixStart, tracks, audioMixVersion)
    : undefined, [audioMixActive, audioMixStart, audioMixVersion, clip.id, config?.mediaBaseUrl, tracks]);

  useEffect(() => () => {
    if (seekSafetyTimer.current !== undefined) window.clearTimeout(seekSafetyTimer.current);
    if (assistedSeekTimer.current !== undefined) window.clearTimeout(assistedSeekTimer.current);
    if (bufferingTimer.current !== undefined) window.clearTimeout(bufferingTimer.current);
    if (previewStopTimer.current !== undefined) window.clearTimeout(previewStopTimer.current);
  }, []);

  useEffect(() => {
    seekInFlight.current = false;
    queuedSeek.current = undefined;
    setPlaying(false);
    setBuffering(false);
    setMediaState(playbackMode === "assisted" && !assistedActive ? "ready" : "loading");
    setMediaError(undefined);
  }, [assistedActive, playbackMode, playbackUrl]);

  useEffect(() => {
    if (!playing) return;
    let frame = 0;
    const update = () => {
      if (video.current) {
        const next = playbackMode === "assisted"
          ? streamStart + video.current.currentTime
          : video.current.currentTime;
        setCurrentTime(Math.min(next, clip.duration));
      }
      frame = window.requestAnimationFrame(update);
    };
    frame = window.requestAnimationFrame(update);
    return () => window.cancelAnimationFrame(frame);
  }, [clip.duration, playbackMode, playing, streamStart]);

  function clearBuffering() {
    if (bufferingTimer.current !== undefined) window.clearTimeout(bufferingTimer.current);
    bufferingTimer.current = undefined;
    setBuffering(false);
  }

  function markPlaybackStarted() {
    if (previewStopTimer.current !== undefined) window.clearTimeout(previewStopTimer.current);
    previewStopTimer.current = undefined;
    clearBuffering();
    setPlaying(true);
    setMediaState("ready");
    const audio = mixedAudio.current;
    const element = video.current;
    if (usesExternalAudioMix && audioMixActive && audio && element && wantsToPlay.current) {
      if (audio.paused) void audio.play().catch(() => undefined);
    }
  }

  function playbackFailed(reason: unknown) {
    if (reason instanceof DOMException && reason.name === "AbortError") return;
    if (reason instanceof Error && reason.name === "AbortError") return;
    wantsToPlay.current = false;
    setPlaying(false);
    setMediaError(reason instanceof Error ? reason.message : String(reason));
  }

  function playVideoOnly() {
    const element = video.current;
    if (!element || !wantsToPlay.current) return;
    void element.play()
      .then(() => {
        if (wantsToPlay.current && !element.paused) markPlaybackStarted();
      })
      .catch(playbackFailed);
  }

  function playWithAudioMix() {
    const element = video.current;
    const audio = mixedAudio.current;
    if (!element || !audio || !wantsToPlay.current) return;
    void Promise.all([element.play(), audio.play()])
      .then(() => {
        if (wantsToPlay.current && !element.paused) markPlaybackStarted();
      })
      .catch(playbackFailed);
  }

  function beginPlayback() {
    if (!wantsToPlay.current) return;
    if (!usesExternalAudioMix || !tracks.length) {
      playVideoOnly();
      return;
    }
    if (!audioMixActive || !mixedAudio.current || mixedAudio.current.readyState < HTMLMediaElement.HAVE_FUTURE_DATA) {
      playWhenAudioMixReady.current = true;
      setBuffering(true);
      setAudioMixStart(video.current?.currentTime ?? currentTime);
      setAudioMixVersion((value) => value + 1);
      setAudioMixActive(true);
      return;
    }
    playWithAudioMix();
  }

  function applySourceSeek(time: number) {
    const element = video.current;
    if (!element) return;
    seekInFlight.current = true;
    element.currentTime = time;
    if (seekSafetyTimer.current !== undefined) window.clearTimeout(seekSafetyTimer.current);
    seekSafetyTimer.current = window.setTimeout(() => {
      seekInFlight.current = false;
      if (queuedSeek.current !== undefined) {
        const queued = queuedSeek.current;
        queuedSeek.current = undefined;
        applySourceSeek(queued);
      }
    }, 1000);
  }

  function freezeCurrentFrame() {
    const element = video.current;
    const canvas = frozenFrame.current;
    if (!element || !canvas || !element.videoWidth || !element.videoHeight) return;
    try {
      canvas.width = element.videoWidth;
      canvas.height = element.videoHeight;
      canvas.getContext("2d")?.drawImage(element, 0, 0, canvas.width, canvas.height);
      setShowFrozenFrame(true);
    } catch {
      setShowFrozenFrame(false);
    }
  }

  function stopAssistedPreviewWhenPaused() {
    if (playbackMode !== "assisted" || wantsToPlay.current) return;
    if (previewStopTimer.current !== undefined) window.clearTimeout(previewStopTimer.current);
    previewStopTimer.current = window.setTimeout(() => {
      previewStopTimer.current = undefined;
      if (wantsToPlay.current) return;
      freezeCurrentFrame();
      setAssistedActive(false);
    }, 120);
  }

  function restartAssistedStream(time: number, playWhenReady = false) {
    if (assistedSeekTimer.current !== undefined) window.clearTimeout(assistedSeekTimer.current);
    freezeCurrentFrame();
    assistedSeekTimer.current = window.setTimeout(() => {
      playAfterSeek.current = playWhenReady;
      setStreamStart(time);
      setStreamVersion((value) => value + 1);
      setAssistedActive(true);
    }, playWhenReady ? 0 : 180);
  }

  function seek(time: number) {
    const next = Math.max(0, Math.min(time, clip.duration));
    setCurrentTime(next);
    if (playbackMode === "assisted") {
      restartAssistedStream(next);
    } else if (seekInFlight.current) {
      queuedSeek.current = next;
    } else {
      applySourceSeek(next);
    }
  }

  function pauseForPrecision() {
    wantsToPlay.current = false;
    playAfterSeek.current = false;
    playWhenAudioMixReady.current = false;
    mixedAudio.current?.pause();
    setAudioMixActive(false);
    if (playbackMode === "assisted" && assistedActive) {
      freezeCurrentFrame();
      setAssistedActive(false);
    } else {
      video.current?.pause();
    }
  }

  function jumpToInPoint() {
    const resumePlayback = wantsToPlay.current || playing;
    if (playbackMode === "assisted") {
      pauseForPrecision();
      setCurrentTime(start);
      if (resumePlayback) {
        wantsToPlay.current = true;
        restartAssistedStream(start, true);
      } else {
        restartAssistedStream(start);
      }
      return;
    }
    if (usesExternalAudioMix && resumePlayback) {
      mixedAudio.current?.pause();
      setAudioMixActive(false);
      video.current?.pause();
      wantsToPlay.current = true;
      playAfterSeek.current = true;
    }
    seek(start);
  }

  function togglePlayback() {
    if (playbackMode === "assisted" && !assistedActive) {
      wantsToPlay.current = true;
      playAfterSeek.current = true;
      restartAssistedStream(currentTime, true);
      return;
    }
    const element = video.current;
    if (!element) return;
    if (wantsToPlay.current || !element.paused) {
      pauseForPrecision();
      return;
    }
    wantsToPlay.current = true;
    if (currentTime < start || currentTime >= end - frameStep / 2) {
      setCurrentTime(start);
      playAfterSeek.current = true;
      if (playbackMode === "assisted") restartAssistedStream(start, true);
      else if (seekInFlight.current) queuedSeek.current = start;
      else applySourceSeek(start);
      return;
    }
    beginPlayback();
  }

  function changeStart(value: number) {
    const next = Math.max(0, Math.min(value, end - frameStep));
    setStart(next);
    if (currentTime < next) {
      pauseForPrecision();
      seek(next);
    }
  }

  function changeEnd(value: number) {
    const next = Math.min(clip.duration, Math.max(value, start + frameStep));
    setEnd(next);
    if (currentTime > next) {
      pauseForPrecision();
      seek(next);
    }
  }

  function setInAtPlayhead() {
    changeStart(Math.min(currentTime, end - frameStep));
  }

  function setOutAtPlayhead() {
    changeEnd(Math.max(currentTime, start + frameStep));
  }

  function toggleAudioTrack(streamIndex: number) {
    pauseForPrecision();
    if (playbackMode === "assisted") {
      freezeCurrentFrame();
      setStreamStart(currentTime);
    }
    setTracks((current) => current.includes(streamIndex)
      ? current.filter((index) => index !== streamIndex)
      : [...current, streamIndex]);
  }

  function togglePlaybackBackend() {
    pauseForPrecision();
    clearBuffering();
    setAudioMixActive(false);
    setMediaState("loading");
    if (playbackMode === "assisted") {
      allowAutomaticFallback.current = false;
      setPlaybackMode("source");
    } else if (config?.mediaBaseUrl) {
      setStreamStart(currentTime);
      setStreamVersion((value) => value + 1);
      setAssistedActive(true);
      setPlaybackMode("assisted");
    }
  }

  function timelineTime(event: ReactPointerEvent<HTMLDivElement>) {
    const bounds = event.currentTarget.getBoundingClientRect();
    return ((event.clientX - bounds.left) / Math.max(bounds.width, 1)) * clip.duration;
  }

  function startTimelineScrub(event: ReactPointerEvent<HTMLDivElement>) {
    timelineScrubbing.current = true;
    timelineWasPlaying.current = wantsToPlay.current || playing;
    event.currentTarget.setPointerCapture(event.pointerId);
    if (playbackMode === "assisted") pauseForPrecision();
    if (usesExternalAudioMix) {
      playWhenAudioMixReady.current = false;
      mixedAudio.current?.pause();
      setAudioMixActive(false);
    }
    seek(timelineTime(event));
  }

  function handleSeeked() {
    clearBuffering();
    setMediaState("ready");
    if (playbackMode !== "assisted") {
      if (seekSafetyTimer.current !== undefined) window.clearTimeout(seekSafetyTimer.current);
      if (queuedSeek.current !== undefined) {
        const queued = queuedSeek.current;
        queuedSeek.current = undefined;
        applySourceSeek(queued);
        return;
      }
      seekInFlight.current = false;
    }
    if (playAfterSeek.current) {
      playAfterSeek.current = false;
      beginPlayback();
    }
  }

  function handlePlaybackError(element: HTMLVideoElement) {
    wantsToPlay.current = false;
    playAfterSeek.current = false;
    setPlaying(false);
    const code = element.error?.code;
    if (playbackMode === "source" && config?.mediaBaseUrl && allowAutomaticFallback.current) {
      allowAutomaticFallback.current = false;
      setStreamStart(currentTime);
      setStreamVersion((value) => value + 1);
      setAssistedActive(true);
      setPlaybackMode("assisted");
      return;
    }
    if (playbackMode === "source" && clip.previewStatus === "ready") {
      setPlaybackMode("proxy");
      return;
    }
    setMediaState("error");
    setMediaError(code ? `The media engine could not decode this file (media error ${code}).` : "The media engine could not decode this file.");
  }

  function handleAudioMixError() {
    playWhenAudioMixReady.current = false;
    setAudioMixActive(false);
    wantsToPlay.current = false;
    video.current?.pause();
    setPlaying(false);
    setMediaState("error");
    setMediaError("The selected preview audio tracks could not be opened. The original video backend is still selected.");
  }

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (target?.matches("input, textarea, select, [contenteditable=true]")) return;
      if (event.code === "Space") {
        event.preventDefault();
        togglePlayback();
      } else if (event.key === "ArrowLeft") {
        event.preventDefault();
        pauseForPrecision();
        seek(currentTime - (event.shiftKey ? 1 : frameStep));
      } else if (event.key === "ArrowRight") {
        event.preventDefault();
        pauseForPrecision();
        seek(currentTime + (event.shiftKey ? 1 : frameStep));
      } else if (event.key.toLowerCase() === "i") {
        event.preventDefault();
        setInAtPlayhead();
      } else if (event.key.toLowerCase() === "o") {
        event.preventDefault();
        setOutAtPlayhead();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  return (
    <div className="editor">
      <div className="editor-heading">
        <div>
          <span className="eyebrow">Editing recording</span>
          <h1>{clip.name}</h1>
          <p>{clip.width}×{clip.height} <b>·</b> {clip.fps.toFixed(2)} fps <b>·</b> {clip.videoCodec.toUpperCase()} <b>·</b> {formatBytes(clip.size)}</p>
        </div>
        <div className="editor-actions">
          <span className="output-pill">Output&nbsp; {config?.export.width || 1920}×{config?.export.height || 1080} / {config?.export.fps || 120} fps</span>
          <button className="delete-button" disabled={busy || Boolean(activeJob)} title={activeJob ? "Wait for the active export to finish" : "Delete this local clip"} onClick={onDelete}>Delete clip</button>
        </div>
      </div>

      <div className="editor-content">
        <div className="preview-column">
          <div className="preview-stage">
            {playbackUrl ? (
              <video
                key={playbackUrl}
                ref={video}
                src={playbackUrl}
                crossOrigin="anonymous"
                poster={clip.previewStatus === "ready" ? api.thumbnailUrl(clip, config) : undefined}
                preload="auto"
                playsInline
                muted={muted || usesExternalAudioMix}
                onClick={togglePlayback}
                onLoadedData={() => {
                  clearBuffering();
                  setShowFrozenFrame(false);
                  setMediaState("ready");
                  setMediaError(undefined);
                  stopAssistedPreviewWhenPaused();
                }}
                onCanPlay={() => {
                  clearBuffering();
                  setShowFrozenFrame(false);
                  setMediaState("ready");
                  if (playAfterSeek.current) {
                    playAfterSeek.current = false;
                    beginPlayback();
                  }
                  stopAssistedPreviewWhenPaused();
                }}
                onPlay={markPlaybackStarted}
                onPlaying={markPlaybackStarted}
                onPause={() => {
                  mixedAudio.current?.pause();
                  setPlaying(false);
                }}
                onWaiting={(event) => {
                  if (event.currentTarget.paused || bufferingTimer.current !== undefined) return;
                  mixedAudio.current?.pause();
                  bufferingTimer.current = window.setTimeout(() => setBuffering(true), 250);
                }}
                onSeeked={handleSeeked}
                onTimeUpdate={(event) => {
                  const time = playbackMode === "assisted"
                    ? streamStart + event.currentTarget.currentTime
                    : event.currentTarget.currentTime;
                  setCurrentTime(time);
                  if (!event.currentTarget.paused && time >= end - frameStep / 2) {
                    pauseForPrecision();
                    setCurrentTime(end);
                  }
                }}
                onEnded={() => {
                  wantsToPlay.current = false;
                  playAfterSeek.current = false;
                  playWhenAudioMixReady.current = false;
                  mixedAudio.current?.pause();
                  setAudioMixActive(false);
                  setPlaying(false);
                }}
                onError={(event) => handlePlaybackError(event.currentTarget)}
              />
            ) : !showFrozenFrame ? (
              <div className="preview-message failed"><strong>Playback unavailable</strong><span>No local playback route is available.</span></div>
            ) : null}
            {audioMixUrl && <audio
              key={audioMixUrl}
              ref={mixedAudio}
              src={audioMixUrl}
              crossOrigin="anonymous"
              preload="auto"
              muted={muted}
              onCanPlay={() => {
                if (!playWhenAudioMixReady.current) return;
                playWhenAudioMixReady.current = false;
                clearBuffering();
                playWithAudioMix();
              }}
              onError={handleAudioMixError}
            />}
            <canvas ref={frozenFrame} className={`frozen-frame ${showFrozenFrame ? "visible" : ""}`} />
            {mediaState === "loading" && !showFrozenFrame && <div className="media-overlay"><i /><span>{playbackMode === "assisted" ? "Starting FFmpeg-assisted playback…" : "Opening original recording…"}</span></div>}
            {mediaState === "error" && <div className="media-overlay error"><strong>Playback unavailable</strong><span>{mediaError}</span>{playbackMode !== "source" && <button onClick={() => setPlaybackMode("source")}>Try original recording</button>}{playbackMode === "source" && clip.previewStatus === "ready" && <button onClick={() => setPlaybackMode("proxy")}>Use cached proxy</button>}</div>}
            {buffering && mediaState === "ready" && <div className="buffering-pill"><i />Buffering</div>}
            {showFrozenFrame && <div className="buffering-pill"><i />Seeking</div>}
          </div>
          {(playbackUrl || playbackMode === "assisted") && <div className="transport" aria-label="Playback controls">
            <button title="Jump to clip in point" onClick={jumpToInPoint}>│◀</button>
            <button className="transport-play" title="Play or pause (Space)" onClick={togglePlayback}>{playing ? "Ⅱ" : "▶"}</button>
            <span className="transport-time"><strong>{formatDuration(currentTime)}</strong> / {formatDuration(clip.duration)}</span>
            <button className={`playback-backend ${playbackMode}`} title={config?.mediaBaseUrl ? "Switch playback backend" : "Current playback backend"} onClick={togglePlaybackBackend}>{playbackMode === "assisted" ? "FFmpeg assisted" : playbackMode === "proxy" ? "Cached proxy" : "Original"}</button>
            <span className="transport-spacer" />
            <button title="Set in point (I)" onClick={setInAtPlayhead}>I&nbsp; In</button>
            <button title="Set out point (O)" onClick={setOutAtPlayhead}>O&nbsp; Out</button>
            <button title={muted ? "Unmute" : "Mute"} onClick={() => setMuted((value) => !value)}>{muted ? "Muted" : "Audio"}</button>
          </div>}
          <section className="trim-card">
            <div className="card-heading">
              <div><span className="step">01</span><div><strong>Trim</strong><small>Select the moment worth keeping</small></div></div>
              <span className="selection-length">{formatDuration(end - start)} selected</span>
            </div>
            <div
              className="timeline"
              onPointerDown={startTimelineScrub}
              onPointerMove={(event) => timelineScrubbing.current && seek(timelineTime(event))}
              onPointerUp={(event) => {
                if (!timelineScrubbing.current) return;
                timelineScrubbing.current = false;
                const time = timelineTime(event);
                if (playbackMode === "assisted" && timelineWasPlaying.current) {
                  wantsToPlay.current = true;
                  restartAssistedStream(time, true);
                } else if (usesExternalAudioMix && timelineWasPlaying.current) {
                  video.current?.pause();
                  wantsToPlay.current = true;
                  playAfterSeek.current = true;
                  seek(time);
                } else {
                  seek(time);
                }
                timelineWasPlaying.current = false;
                if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
              }}
              onPointerCancel={() => {
                timelineScrubbing.current = false;
                if (playbackMode === "assisted" && timelineWasPlaying.current) {
                  wantsToPlay.current = true;
                  restartAssistedStream(currentTime, true);
                } else if (usesExternalAudioMix && timelineWasPlaying.current) {
                  video.current?.pause();
                  wantsToPlay.current = true;
                  playAfterSeek.current = true;
                  seek(currentTime);
                }
                timelineWasPlaying.current = false;
              }}
            >
              <div className="timeline-track" />
              <div className="selection" style={{ left: `${(start / clip.duration) * 100}%`, right: `${100 - (end / clip.duration) * 100}%` }} />
              <div className="playhead" style={{ left: `${(currentTime / Math.max(clip.duration, frameStep)) * 100}%` }}><span>{formatDuration(currentTime)}</span></div>
              <input className="trim-start" aria-label="Trim start" type="range" min="0" max={clip.duration} step={frameStep} value={start} onPointerDown={(event) => event.stopPropagation()} onChange={(event) => changeStart(Number(event.currentTarget.value))} />
              <input className="trim-end" aria-label="Trim end" type="range" min="0" max={clip.duration} step={frameStep} value={end} onPointerDown={(event) => event.stopPropagation()} onChange={(event) => changeEnd(Number(event.currentTarget.value))} />
            </div>
            <div className="timeline-hints"><span>Click or drag to scrub</span><span>←/→ frame · Shift + ←/→ 1 second · I/O set points · Space play</span></div>
            <div className="time-inputs">
              <label><span>In point</span><input type="number" min="0" max={end - frameStep} step={frameStep} value={start.toFixed(3)} onChange={(event) => changeStart(Number(event.target.value))} /><small>seconds</small></label>
              <button onClick={() => { setStart(0); setEnd(clip.duration); }}>Reset</button>
              <label><span>Out point</span><input type="number" min={start + frameStep} max={clip.duration} step={frameStep} value={end.toFixed(3)} onChange={(event) => changeEnd(Number(event.target.value))} /><small>seconds</small></label>
            </div>
          </section>
        </div>

        <div className="control-column">
          <section className="audio-card">
            <div className="card-heading">
              <div><span className="step">02</span><div><strong>Audio mix</strong><small>Selected tracks are mixed for reliable playback</small></div></div>
              <span className="selection-length">{tracks.length} of {clip.audioTracks.length} tracks</span>
            </div>
            <div className="track-grid">
              {clip.audioTracks.map((track) => {
                const selected = tracks.includes(track.streamIndex);
                return <button key={track.streamIndex} className={`track ${selected ? "selected" : ""}`} onClick={() => toggleAudioTrack(track.streamIndex)}>
                  <span className="check">{selected ? "✓" : ""}</span>
                  <span className="track-icon">≋</span>
                  <span><strong>{trackName(track, config?.audioTrackLabels)}</strong><small>Track {track.ordinal + 1} · {track.codec.toUpperCase()} · {track.channelLayout || `${track.channels} channels`}</small></span>
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
          {completedJobs.length > 0 && (
            <section className="versions-card">
              <div className="versions-heading">
                <div><strong>Published versions</strong><small>Each re-publish is kept separately</small></div>
                <span>{completedJobs.length} version{completedJobs.length === 1 ? "" : "s"}</span>
              </div>
              <div className="version-list">
                {completedJobs.map((job, index) => {
                  const selection = job.selection;
                  const audioNames = selection?.audioStreamIndexes.map((streamIndex) => {
                    const track = clip.audioTracks.find((item) => item.streamIndex === streamIndex);
                    return track ? trackName(track, config?.audioTrackLabels) : `Track ${streamIndex}`;
                  });
                  return (
                    <div className="version-row" key={job.id}>
                      <div className="version-copy">
                        <div><strong>{index === 0 ? "Latest version" : `Version ${completedJobs.length - index}`}</strong><span>{new Date(job.publishedAt || job.createdAt).toLocaleString()}</span></div>
                        <small>{selection
                          ? `${formatDuration(selection.start)}–${formatDuration(selection.end)} · ${audioNames?.length ? audioNames.join(" + ") : "No audio"}`
                          : "Edit settings unavailable for this older version"} · {expiryLabel(job.expiresAt)}</small>
                      </div>
                      <div className="version-actions">
                        <button onClick={() => job.url && void api.openExternal(job.url)}>Open</button>
                        <button disabled={busy} onClick={() => void api.copyText(job.url!)}>Copy</button>
                        <button className="version-delete" disabled={busy} onClick={() => onDeleteVersion(job)}>Delete</button>
                      </div>
                    </div>
                  );
                })}
              </div>
            </section>
          )}
        </div>
      </div>
    </div>
  );
}
