import { useCallback, useEffect, useRef, useState } from "react";
import { collectItem } from "../../api/commands";
import { remoteStreamUrl } from "../../api/stream";
import type { ResultItem } from "../../api/types";
import { playExclusive } from "../../preview/audioBus";
import { useSetting } from "../../settings/useSettings";

type CollectState = "idle" | "working" | "done" | "duplicate" | "error";

function fmtDuration(ms: number | null): string | null {
  if (!ms) return null;
  const s = Math.round(ms / 1000);
  return s >= 60 ? `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}` : `${s}s`;
}

export function ResultTile({ item }: { item: ResultItem }) {
  const [collect, setCollect] = useState<CollectState>("idle");
  const [error, setError] = useState<string | null>(null);
  const showBadge = useSetting("grid.show_source_badges", true);

  const onCollect = useCallback(async () => {
    if (collect === "working") return;
    setCollect("working");
    setError(null);
    try {
      const outcome = await collectItem(item);
      setCollect(outcome.was_duplicate ? "duplicate" : "done");
    } catch (e) {
      setCollect("error");
      setError(String(e));
    }
  }, [collect, item]);

  const collectLabel = {
    idle: "+ Collect",
    working: "Collecting…",
    done: "✓ Collected",
    duplicate: "✓ Already saved",
    error: "Retry",
  }[collect];

  return (
    <div className={`tile tile-${item.preview_kind}`} title={error ?? item.title}>
      {item.preview_kind === "audio_stream" ? (
        <AudioPreview item={item} />
      ) : (
        <VisualPreview item={item} />
      )}
      <div className="tile-meta">
        <div className="tile-title" title={item.title}>
          {item.title}
        </div>
        <div className="tile-sub">
          {showBadge && <span className={`badge badge-${item.source}`}>{item.source}</span>}
          {fmtDuration(item.duration_ms) && (
            <span className="duration">{fmtDuration(item.duration_ms)}</span>
          )}
          <button
            className={`collect collect-${collect}`}
            onClick={onCollect}
            disabled={collect === "working"}
          >
            {collectLabel}
          </button>
        </div>
        {item.license && <div className="tile-license">{item.license}</div>}
      </div>
    </div>
  );
}

/** inline streaming audio row: play/pause + progress, via wzstream only */
function AudioPreview({ item }: { item: ResultItem }) {
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const [playing, setPlaying] = useState(false);
  const [progress, setProgress] = useState(0);
  const volume = useSetting("preview.audio_volume", 0.8);

  const toggle = useCallback(() => {
    if (!item.preview_stream_url) return;
    let el = audioRef.current;
    if (!el) {
      el = document.createElement("audio"); // src is ALWAYS a wzstream URL
      el.preload = "none";
      el.src = remoteStreamUrl(item.source, item.preview_stream_url);
      el.onplay = () => setPlaying(true);
      el.onpause = () => setPlaying(false);
      el.onended = () => {
        setPlaying(false);
        setProgress(0);
      };
      el.ontimeupdate = () => {
        if (el!.duration > 0) setProgress(el!.currentTime / el!.duration);
      };
      audioRef.current = el;
    }
    el.volume = volume;
    if (el.paused) {
      playExclusive(el);
    } else {
      el.pause();
    }
  }, [item, volume]);

  useEffect(() => {
    return () => {
      audioRef.current?.pause();
      audioRef.current = null;
    };
  }, []);

  return (
    <button className="audio-preview" onClick={toggle}>
      <span className="audio-icon">{playing ? "⏸" : "▶"}</span>
      <span className="audio-bar">
        <span className="audio-bar-fill" style={{ width: `${progress * 100}%` }} />
      </span>
    </button>
  );
}

/** thumbnail that swaps to a looping preview on hover (video or animated image) */
function VisualPreview({ item }: { item: ResultItem }) {
  const hoverToPlay = useSetting("preview.hover_to_play", true);
  const hoverDelay = useSetting("preview.hover_delay_ms", 120);
  const muted = useSetting("preview.mute_video_previews", true);
  const [active, setActive] = useState(false);
  const timer = useRef<number | undefined>(undefined);

  const previewUrl = item.preview_stream_url
    ? remoteStreamUrl(item.source, item.preview_stream_url)
    : null;
  // thumbnails may load straight from https (CSP img-src); media never does
  const thumbUrl = item.thumbnail_url ?? null;
  const isVideo = item.preview_kind === "video_loop" || item.preview_kind === "poster_loop";
  const ratio =
    item.width && item.height ? `${item.width} / ${item.height}` : "16 / 10";

  const enter = () => {
    if (!hoverToPlay || !previewUrl) return;
    timer.current = window.setTimeout(() => setActive(true), hoverDelay);
  };
  const leave = () => {
    window.clearTimeout(timer.current);
    setActive(false);
  };

  return (
    <div
      className="visual-preview"
      style={{ aspectRatio: ratio }}
      onMouseEnter={enter}
      onMouseLeave={leave}
      onClick={() => previewUrl && setActive((a) => !a)}
    >
      {active && previewUrl ? (
        isVideo ? (
          <video src={previewUrl} autoPlay loop muted={muted} playsInline />
        ) : (
          <img src={previewUrl} alt={item.title} loading="lazy" />
        )
      ) : thumbUrl ? (
        <img src={thumbUrl} alt={item.title} loading="lazy" />
      ) : (
        <div className="visual-placeholder">{item.asset_type}</div>
      )}
      {item.asset_type === "green_screen" && <span className="gs-tag">GS</span>}
    </div>
  );
}
